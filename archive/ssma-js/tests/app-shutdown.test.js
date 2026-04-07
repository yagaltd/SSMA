import { afterEach, describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import fs from "node:fs";
import path from "node:path";
import { tmpdir } from "node:os";
import { createServer } from "../src/app.js";
import { loadConfig } from "../src/config/env.js";

function makeTempPath(prefix) {
  return path.join(
    tmpdir(),
    `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}.json`,
  );
}

async function listen(server) {
  await new Promise((resolve) => server.listen(0, resolve));
  return `http://127.0.0.1:${server.address().port}`;
}

async function waitForSocketOpen(ws) {
  await new Promise((resolve, reject) => {
    ws.once("open", resolve);
    ws.once("error", reject);
  });
}

async function waitForSocketClose(ws) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("WS_CLOSE_TIMEOUT")), 2000);
    ws.once("close", (code, reason) => {
      clearTimeout(timer);
      resolve({ code, reason: reason.toString() });
    });
    ws.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
  });
}

describe("createServer shutdown", () => {
  const tempFiles = [];

  afterEach(() => {
    while (tempFiles.length > 0) {
      fs.rmSync(tempFiles.pop(), { force: true });
    }
  });

  it("closes upgraded websocket clients during server.close", async () => {
    const config = loadConfig();
    const storePath = makeTempPath("ssma-shutdown-intents");
    const userStorePath = makeTempPath("ssma-shutdown-users");
    tempFiles.push(storePath, userStorePath);

    config.server.port = 0;
    config.optimistic.storePath = storePath;
    config.auth.userStorePath = userStorePath;
    config.backend.url = "";

    const server = createServer(config);
    const baseUrl = await listen(server);
    const ws = new WebSocket(
      `${baseUrl.replace("http://", "ws://")}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0`,
    );

    try {
      await waitForSocketOpen(ws);

      const socketClosed = waitForSocketClose(ws);
      await new Promise((resolve) => server.close(resolve));
      const closeInfo = await socketClosed;

      expect(closeInfo.code).toBe(1001);
      expect(closeInfo.reason).toBe("SERVER_SHUTDOWN");
    } finally {
      if (server.listening) {
        await new Promise((resolve) => server.close(resolve));
      }
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.terminate();
      }
    }
  });
});
