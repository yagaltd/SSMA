import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import { createServer } from "../src/app.js";
import { loadConfig } from "../src/config/env.js";
import { createToyBackendServer } from "../examples/toy-backend/server.mjs";
import path from "node:path";
import fs from "node:fs";
import { tmpdir } from "node:os";

const TEST_TIMEOUT = 10000;

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function parseSseFrames(buffer, events) {
  let index;
  while ((index = buffer.indexOf("\n\n")) >= 0) {
    const frame = buffer.slice(0, index);
    buffer = buffer.slice(index + 2);
    const lines = frame.split("\n");
    let type = "message";
    const dataLines = [];
    for (const line of lines) {
      if (line.startsWith("event:")) type = line.slice(6).trim();
      if (line.startsWith("data:")) dataLines.push(line.slice(5).trim());
    }
    if (dataLines.length) {
      const raw = dataLines.join("\n");
      let data;
      try {
        data = JSON.parse(raw);
      } catch {
        data = raw;
      }
      events.push({ type, data });
    }
  }
  return buffer;
}

async function openSse(baseUrl) {
  const controller = new AbortController();
  const response = await fetch(`${baseUrl}/optimistic/events`, {
    headers: { accept: "text/event-stream" },
    signal: controller.signal,
  });
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const events = [];
  let buffer = "";
  let closed = false;

  const loop = (async () => {
    try {
      while (!closed) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder
          .decode(value, { stream: true })
          .replace(/\r\n/g, "\n");
        buffer = parseSseFrames(buffer, events);
      }
    } catch {
      // stream ended
    }
  })();

  return {
    async waitFor(type, timeoutMs = TEST_TIMEOUT) {
      const start = Date.now();
      while (Date.now() - start < timeoutMs) {
        const hit = events.find((evt) => evt.type === type);
        if (hit) return hit;
        await delay(20);
      }
      throw new Error(`Timed out waiting for SSE event: ${type}`);
    },
    async waitForAny(types, timeoutMs = TEST_TIMEOUT) {
      const wanted = new Set(types);
      const start = Date.now();
      while (Date.now() - start < timeoutMs) {
        const hit = events.find((evt) => wanted.has(evt.type));
        if (hit) return hit;
        await delay(20);
      }
      throw new Error(
        `Timed out waiting for SSE events: ${Array.from(wanted).join(", ")}. Seen: ${events.map((evt) => evt.type).join(", ")}`,
      );
    },
    close() {
      closed = true;
      controller.abort();
      return loop;
    },
  };
}

async function wsConnect(
  baseUrl,
  {
    role = "leader",
    site = "default",
    subprotocol = "1.0.0",
    cookie = "",
  } = {},
) {
  const wsBase = baseUrl
    .replace("http://", "ws://")
    .replace("https://", "wss://");
  const ws = new WebSocket(
    `${wsBase}/optimistic/ws?role=${encodeURIComponent(role)}&site=${encodeURIComponent(site)}&subprotocol=${encodeURIComponent(subprotocol)}`,
    {
      headers: cookie ? { Cookie: cookie } : {},
    },
  );

  const inbox = [];
  ws.on("message", (raw) => {
    try {
      inbox.push(JSON.parse(raw.toString()));
    } catch {
      // ignore malformed frame for test helper
    }
  });

  await new Promise((resolve, reject) => {
    ws.once("open", resolve);
    ws.once("error", reject);
  });

  return {
    ws,
    send(payload) {
      ws.send(typeof payload === "string" ? payload : JSON.stringify(payload));
    },
    async waitFor(type, timeoutMs = TEST_TIMEOUT) {
      const start = Date.now();
      while (Date.now() - start < timeoutMs) {
        const index = inbox.findIndex((msg) => msg.type === type);
        if (index >= 0) {
          return inbox.splice(index, 1)[0];
        }
        await delay(20);
      }
      throw new Error(`Timed out waiting for WS type: ${type}`);
    },
    close() {
      ws.close();
    },
  };
}

describe("SSMA <-> backend simulation", () => {
  let ssmaServer;
  let backendServer;
  let ssmaBase;
  let backendBase;
  let storePath;
  let userStorePath;

  beforeAll(async () => {
    backendServer = createToyBackendServer();
    await new Promise((resolve) => backendServer.listen(0, resolve));
    backendBase = `http://127.0.0.1:${backendServer.address().port}`;

    const config = loadConfig();
    storePath = path.join(tmpdir(), `ssma-e2e-intents-${Date.now()}.json`);
    userStorePath = path.join(tmpdir(), `ssma-e2e-users-${Date.now()}.json`);
    config.server.port = 0;
    config.optimistic.storePath = storePath;
    config.auth.userStorePath = userStorePath;
    config.backend.url = backendBase;
    config.backend.internalToken = "test-token";
    config.optimistic.requireAuthForWrites = true;
    ssmaServer = createServer(config);
    await new Promise((resolve) => ssmaServer.listen(0, resolve));
    ssmaBase = `http://127.0.0.1:${ssmaServer.address().port}`;
  }, 20000);

  afterAll(async () => {
    ssmaServer.closeAllConnections?.();
    backendServer.closeAllConnections?.();
    await new Promise((resolve) => ssmaServer.close(resolve));
    await new Promise((resolve) => backendServer.close(resolve));
    if (storePath) fs.rmSync(storePath, { force: true });
    if (userStorePath) fs.rmSync(userStorePath, { force: true });
  }, 20000);

  it(
    "Scenario A1: handshake + empty replay",
    async () => {
      const ws = await wsConnect(ssmaBase, {
        role: "leader",
        subprotocol: "1.0.0",
      });
      const hello = await ws.waitFor("hello");
      expect(hello.subprotocol).toBe("1.0.0");
      const replay = await ws.waitFor("replay");
      expect(Array.isArray(replay.intents)).toBe(true);
      expect(typeof replay.cursor).toBe("number");
      ws.close();
    },
    TEST_TIMEOUT,
  );

  it(
    "Scenario A2: non-empty replay + monotonic cursor",
    async () => {
      const register = await fetch(`${ssmaBase}/auth/register`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          email: `e2e-replay-${Date.now()}@ssma.local`,
          password: "password-123",
          name: "Replay User",
        }),
      });
      const cookie = register.headers.get("set-cookie");
      expect(cookie).toBeTruthy();

      const seeded = await wsConnect(ssmaBase, { role: "leader", cookie });
      await seeded.waitFor("hello");
      await seeded.waitFor("replay");
      seeded.send({
        type: "intent.batch",
        intents: [
          {
            id: "i-replay-0001",
            intent: "TODO_CREATE",
            payload: { id: "todo-replay-1", title: "replay seed" },
            meta: { clock: Date.now(), channels: ["global"] },
          },
        ],
      });
      const seededAck = await seeded.waitFor("ack");
      const firstCursor = seededAck.intents[0].logSeq;
      expect(typeof firstCursor).toBe("number");
      seeded.close();

      const resumed = await wsConnect(ssmaBase, {
        role: "leader",
        cookie,
        subprotocol: "1.0.0",
      });
      await resumed.waitFor("hello");
      const resumedReplay = await resumed.waitFor("replay");
      expect(Array.isArray(resumedReplay.intents)).toBe(true);
      expect(resumedReplay.cursor).toBeGreaterThanOrEqual(firstCursor);
      resumed.close();
    },
    TEST_TIMEOUT,
  );

  it("Scenario B/C/D: write flow, idempotency, and unauthorized rejection", async () => {
    const register = await fetch(`${ssmaBase}/auth/register`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        email: `e2e-${Date.now()}@ssma.local`,
        password: "password-123",
        name: "E2E User",
      }),
    });
    expect(register.ok).toBe(true);
    const cookie = register.headers.get("set-cookie");
    expect(cookie).toBeTruthy();

    const sse = await openSse(ssmaBase);
    await sse.waitFor("ready");

    const ws = await wsConnect(ssmaBase, { role: "leader", cookie });
    await ws.waitFor("hello");

    const intent = {
      type: "intent.batch",
      intents: [
        {
          id: "i-1-abcdefg",
          intent: "TODO_CREATE",
          payload: { id: "todo-1", title: "SSMA todo" },
          meta: { clock: Date.now(), channels: ["global"] },
        },
      ],
    };

    ws.send(intent);
    const ack = await ws.waitFor("ack");
    expect(ack.intents[0].id).toBe("i-1-abcdefg");
    expect(ack.intents[0].status).toBe("acked");
    expect(typeof ack.intents[0].logSeq).toBe("number");

    const invalidate = await sse.waitForAny([
      "invalidate",
      "island.invalidate",
    ]);
    if (invalidate.type === "invalidate") {
      expect(Array.isArray(invalidate.data.intents)).toBe(true);
      expect(invalidate.data.intents[0].id).toBe("i-1-abcdefg");
    } else {
      expect(invalidate.data.islandId).toBeTruthy();
    }

    ws.send(intent);
    const ackRetry = await ws.waitFor("ack");
    expect(ackRetry.intents[0].status).toBe("acked");

    const metrics = await fetch(`${backendBase}/metrics`).then((res) =>
      res.json(),
    );
    const count = metrics.applyCountByIntent.find(
      (entry) => entry.id === "i-1-abcdefg",
    )?.count;
    expect(count).toBe(1);

    ws.close();
    await sse.close();

    const unauthWs = await wsConnect(ssmaBase, { role: "leader" });
    await unauthWs.waitFor("hello");
    unauthWs.send(intent);
    const unauthorized = await unauthWs.waitFor("error");
    expect(unauthorized.code).toBe("UNAUTHORIZED");
    unauthWs.close();
  }, 20000);

  it("Scenario E: channel subscribe snapshot", async () => {
    const register = await fetch(`${ssmaBase}/auth/register`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        email: `e2e-snap-${Date.now()}@ssma.local`,
        password: "password-123",
        name: "Snapshot User",
      }),
    });
    const cookie = register.headers.get("set-cookie");

    const ws = await wsConnect(ssmaBase, { role: "leader", cookie });
    await ws.waitFor("hello");

    ws.send({
      type: "intent.batch",
      intents: [
        {
          id: "i-2-abcdefg",
          intent: "TODO_CREATE",
          payload: { id: "todo-2", title: "snapshot seed" },
          meta: { clock: Date.now(), channels: ["global"] },
        },
      ],
    });
    await ws.waitFor("ack");

    ws.send({ type: "channel.subscribe", channel: "global", params: {} });
    const ack = await ws.waitFor("channel.ack");
    expect(ack.status).toBe("ok");
    const snapshot = await ws.waitFor("channel.snapshot");
    expect(snapshot.channel).toBe("global");
    expect(Array.isArray(snapshot.intents)).toBe(true);
    expect(snapshot.intents.length).toBeGreaterThan(0);

    ws.close();
  }, 20000);

  it(
    "Scenario F: subprotocol mismatch",
    async () => {
      const ws = await wsConnect(ssmaBase, {
        role: "leader",
        subprotocol: "2.0.0",
      });
      const error = await ws.waitFor("error");
      expect(error.code).toBe("SUBPROTOCOL_MISMATCH");
      ws.close();
    },
    TEST_TIMEOUT,
  );
});
