import { WebSocketServer } from "ws";
import { URL } from "node:url";
import { randomUUID } from "node:crypto";
import { validateContract } from "../../runtime/ContractRegistry.js";
import { parse as parseCookie } from "cookie";
import { hasRole } from "../../utils/rbac.js";

export class SyncGateway {
  constructor({
    server,
    intentStore,
    eventHub,
    channelRegistry,
    authService,
    authCookieName = "ssma_session",
    logService,
    channelRateLimit,
    allowedOrigins = "*",
    replayWindowMs = 5 * 60 * 1000,
    subprotocol = "1.0.0",
    transport = {},
    monitor = null,
    backendClient = null,
    requireAuthForWrites = false,
  } = {}) {
    this.server = server;
    this.intentStore = intentStore;
    this.eventHub = eventHub;
    this.channelRegistry = channelRegistry;
    this.authService = authService;
    this.authCookieName = authCookieName;
    this.logService = logService;
    this.channelRateLimit = channelRateLimit;
    this.allowedOrigins = allowedOrigins;
    this.replayWindowMs = replayWindowMs;
    this.subprotocol = subprotocol;
    this.monitor = monitor;
    this.backendClient = backendClient;
    this.requireAuthForWrites = requireAuthForWrites;
    this.wss = null;
    this.channelBuckets = new Map();
    this.connections = new Map();
    this.wsConfig = {
      maxBufferedBytes: transport?.ws?.maxBufferedBytes ?? 256 * 1024,
      slowConsumerCloseMs: transport?.ws?.slowConsumerCloseMs ?? 10 * 1000,
    };
    this.islandAccess = new Map(Object.entries(transport?.islandAccess || {}));
    this.metrics = {
      broadcastsTotal: 0,
      broadcastsByType: {},
      lastIslandInvalidateAt: null,
      connectionsAccepted: 0,
      connectionsClosed: 0,
      activeConnections: 0,
      backpressureEvents: 0,
      slowConsumerDrops: 0,
      unauthorizedFiltered: 0,
      latency: { count: 0, sum: 0, max: 0, avg: 0 },
    };
  }

  start() {
    if (!this.server) {
      throw new Error("[SyncGateway] HTTP server is required");
    }

    this.wss = new WebSocketServer({ noServer: true });
    this.server.on("upgrade", (request, socket, head) => {
      try {
        const url = new URL(
          request.url || "/",
          `http://${request.headers.host || "localhost"}`,
        );
        if (url.pathname !== "/optimistic/ws") {
          socket.destroy();
          return;
        }
        this.wss.handleUpgrade(request, socket, head, (ws) => {
          this.handleConnection(ws, url, request);
        });
      } catch (error) {
        console.error("[SyncGateway] Failed to handle upgrade", error);
        socket.destroy();
      }
    });
  }

  async handleConnection(ws, url, request) {
    const role = (url.searchParams.get("role") || "follower").toLowerCase();
    const site = url.searchParams.get("site") || "default";
    const clientSubprotocol =
      url.searchParams.get("subprotocol") || this.subprotocol;
    const cursor = Number(url.searchParams.get("cursor")) || 0;
    if (!this._isSubprotocolCompatible(clientSubprotocol)) {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "SUBPROTOCOL_MISMATCH",
          expected: this.subprotocol,
        }),
      );
      ws.close(4001, "Subprotocol mismatch");
      return;
    }
    const connectionId = randomUUID();
    const user = await this._resolveUser(request);
    const ip = this._getIp(request);
    const context = {
      role,
      connectionId,
      site,
      user,
      ip,
      userAgent: request?.headers?.["user-agent"] || null,
    };

    ws.send(
      JSON.stringify({
        type: "hello",
        role,
        connectionId,
        serverTime: Date.now(),
        subprotocol: this.subprotocol,
      }),
    );
    this._sendReplay(ws, cursor);

    this.channelRegistry?.attachConnection(connectionId, {
      send: (payload) => this._safeSend(ws, payload, context),
      context,
    });
    this.connections.set(connectionId, { ws, context });
    this.metrics.connectionsAccepted += 1;
    this.metrics.activeConnections = this.connections.size;

    ws.on("message", (raw) => {
      this.handleMessage(ws, context, raw).catch((error) => {
        console.error("[SyncGateway] Failed to process message", error);
        this._safeSend(
          ws,
          {
            type: "error",
            code: "SERVER_ERROR",
            message: "Failed to process message",
          },
          context,
        );
      });
    });

    ws.on("close", () => {
      this.eventHub?.publish("connection:closed", { connectionId, role });
      this.channelRegistry?.detachConnection(connectionId);
      this.connections.delete(connectionId);
      this.metrics.connectionsClosed += 1;
      this.metrics.activeConnections = this.connections.size;
    });
  }

  async handleMessage(ws, context, raw) {
    let message;
    try {
      message = JSON.parse(raw.toString());
    } catch (error) {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "INVALID_JSON",
          message: "Payload must be JSON",
        }),
      );
      return;
    }

    if (!this._validateInboundMessage(ws, message)) {
      return;
    }

    switch (message.type) {
      case "intent.batch":
        await this.handleIntentBatch(ws, context, message);
        break;
      case "channel.subscribe":
        await this.handleChannelSubscribe(ws, context, message);
        break;
      case "channel.unsubscribe":
        this.handleChannelUnsubscribe(ws, context, message);
        break;
      case "channel.resync":
        await this.handleChannelResync(ws, context, message);
        break;
      case "channel.command":
        await this.handleChannelCommand(ws, context, message);
        break;
      case "ping":
        ws.send(JSON.stringify({ type: "pong", ts: Date.now() }));
        break;
      default:
        ws.send(
          JSON.stringify({
            type: "error",
            code: "UNKNOWN_TYPE",
            message: `Unsupported type: ${message.type}`,
          }),
        );
    }
  }

  async handleChannelSubscribe(ws, context, message) {
    if (!this.channelRegistry) {
      this._safeSend(
        ws,
        { type: "channel.ack", status: "error", code: "CHANNELS_DISABLED" },
        context,
      );
      return;
    }
    const rate = this._consumeChannelRate(context);
    if (!rate.allowed) {
      this._safeSend(
        ws,
        {
          type: "channel.ack",
          status: "error",
          code: "RATE_LIMITED",
          retryAfterMs: rate.retryAfterMs,
        },
        context,
      );
      this._log(
        "CHANNEL_SUBSCRIBE_RATE_LIMIT",
        {
          channel: message.channel,
          params: message.params || {},
          connectionId: context.connectionId,
          userId: context.user?.id,
          role: context.user?.role,
          site: context.site,
          retryAfterMs: rate.retryAfterMs,
        },
        "warn",
      );
      return;
    }
    const response = await this.channelRegistry.subscribe(
      context.connectionId,
      {
        channel: message.channel,
        params: message.params || {},
        filter: message.filter || null,
      },
    );
    let backendSnapshot = null;
    if (this.backendClient?.supports?.("subscribe")) {
      try {
        const backendResponse = await this.backendClient.subscribe(
          message.channel,
          message.params || {},
          context,
        );
        if (
          backendResponse?.status === "ok" &&
          Array.isArray(backendResponse.snapshot)
        ) {
          backendSnapshot = {
            intents: backendResponse.snapshot,
            cursor: Number(backendResponse.cursor) || response.cursor || 0,
          };
        }
      } catch (error) {
        this._log(
          "CHANNEL_SUBSCRIBE_BACKEND_SNAPSHOT_FAILED",
          {
            channel: message.channel,
            connectionId: context.connectionId,
            error: error.message,
          },
          "warn",
        );
      }
    }

    const { intents, close, ...ack } = response;
    this._safeSend(ws, { type: "channel.ack", ...ack }, context);
    this._log(
      "CHANNEL_SUBSCRIBE",
      {
        channel: message.channel,
        params: message.params || {},
        status: response.status,
        code: response.code,
        connectionId: context.connectionId,
        role: context.user?.role,
        userId: context.user?.id,
        site: context.site,
      },
      response.status === "error" ? "warn" : "info",
    );
    const snapshotIntents = backendSnapshot?.intents || intents;
    if (
      response.status === "ok" &&
      Array.isArray(snapshotIntents)
    ) {
      this._safeSend(
        ws,
        {
          type: "channel.snapshot",
          channel: response.channel,
          intents: snapshotIntents,
          params: response.params,
          cursor: backendSnapshot?.cursor || response.cursor,
        },
        context,
      );
    }
    if (close) {
      this._safeSend(
        ws,
        {
          type: "channel.close",
          channel: message.channel,
          code: close.code,
          reason: close.reason,
          params: message.params || {},
          timestamp: Date.now(),
        },
        context,
      );
    }
  }

  handleChannelUnsubscribe(ws, context, message) {
    if (!this.channelRegistry) {
      this._safeSend(
        ws,
        {
          type: "channel.unsubscribed",
          status: "error",
          code: "CHANNELS_DISABLED",
        },
        context,
      );
      return;
    }
    const result = this.channelRegistry.unsubscribe(context.connectionId, {
      channel: message.channel,
      params: message.params || {},
    });
    this._safeSend(ws, { type: "channel.unsubscribed", ...result }, context);
    if (result.status === "ok") {
      this._safeSend(
        ws,
        {
          type: "channel.close",
          channel: message.channel,
          code: "CLIENT_UNSUBSCRIBED",
          reason: "Client requested unsubscribe",
          params: message.params || {},
          timestamp: Date.now(),
        },
        context,
      );
    }
    this._log("CHANNEL_UNSUBSCRIBE", {
      channel: message.channel,
      params: message.params || {},
      status: result.status,
      connectionId: context.connectionId,
      role: context.user?.role,
      userId: context.user?.id,
      site: context.site,
    });
  }

  broadcast(type, payload = {}) {
    if (!type) return;
    this.metrics.broadcastsTotal += 1;
    this.metrics.broadcastsByType[type] =
      (this.metrics.broadcastsByType[type] || 0) + 1;
    if (type === "island.invalidate") {
      this.metrics.lastIslandInvalidateAt = Date.now();
      console.log(
        `[SyncGateway] WS broadcast island.invalidate #${this.metrics.broadcastsByType[type]}`,
      );
      const latency = this.#computeLatency(payload);
      if (latency !== null) {
        this.#recordLatency(latency);
      }
    }
    const body =
      typeof payload === "object" && payload !== null
        ? { ...payload, type }
        : { type, payload };
    const shouldFilter = type === "island.invalidate";
    for (const { ws, context } of this.connections.values()) {
      if (shouldFilter && !this.#islandAuthorized(context, payload?.islandId)) {
        this.metrics.unauthorizedFiltered += 1;
        continue;
      }
      this._safeSend(ws, body, context);
    }
  }

  async handleChannelResync(ws, context, message) {
    if (!this.channelRegistry) {
      this._safeSend(
        ws,
        { type: "channel.replay", status: "error", code: "CHANNELS_DISABLED" },
        context,
      );
      return;
    }
    const response = await this.channelRegistry.resync(context.connectionId, {
      channel: message.channel,
      cursor: Number(message.cursor) || 0,
      limit: Number(message.limit) || 200,
      params: message.params || {},
    });
    this._safeSend(ws, { type: "channel.replay", ...response }, context);
  }

  async handleChannelCommand(ws, context, message) {
    if (!this.channelRegistry) {
      this._safeSend(
        ws,
        { type: "channel.command", status: "error", code: "CHANNELS_DISABLED" },
        context,
      );
      return;
    }
    const response = await this.channelRegistry.command(context.connectionId, {
      channel: message.channel,
      params: message.params || {},
      command: message.command,
      args: message.args || {},
    });
    this._safeSend(ws, { type: "channel.command", ...response }, context);
    if (response.status === "error" && response.code === "ACCESS_DENIED") {
      this._safeSend(
        ws,
        {
          type: "channel.close",
          channel: message.channel,
          code: "ACCESS_DENIED",
          reason: "Access denied",
          params: message.params || {},
          timestamp: Date.now(),
        },
        context,
      );
    }
  }

  async handleIntentBatch(ws, context, message) {
    if (context.role !== "leader") {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "NOT_LEADER",
          message: "Only leader tabs may flush intents",
        }),
      );
      return;
    }
    if (this.requireAuthForWrites && !context.user?.id) {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "UNAUTHORIZED",
          message: "Authentication required for writes",
        }),
      );
      return;
    }

    const intents = Array.isArray(message.intents) ? message.intents : [];
    if (intents.length === 0) {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "EMPTY_BATCH",
          message: "Batch contained no intents",
        }),
      );
      return;
    }

    const replayed = [];
    const fresh = [];
    for (const intent of intents) {
      const existing = this.intentStore.get(intent.id);
      if (existing && existing.site === context.site) {
        replayed.push(existing);
        continue;
      }
      fresh.push(intent);
    }

    const stored = this.intentStore.append(fresh, context);
    if (stored.length) {
      this._log("INTENT_ACCEPTED", {
        site: context.site,
        connectionId: context.connectionId,
        intents: stored.map((entry) => entry.id),
      });
    }
    const statusById = new Map();
    const backendEvents = [];
    for (const entry of replayed) {
      statusById.set(entry.id, entry.status || "acked");
    }

    for (const entry of stored) {
      statusById.set(entry.id, entry.status || "acked");
    }

    if (stored.length && this.backendClient?.isConfigured?.()) {
      this._log("INTENT_FORWARDED", {
        site: context.site,
        connectionId: context.connectionId,
        intents: stored.map((entry) => entry.id),
      });
      try {
        const backendResult = await this.backendClient.applyIntents(
          stored,
          context,
        );
        const resultList = Array.isArray(backendResult?.results)
          ? backendResult.results
          : [];
        for (const result of resultList) {
          if (!result?.id) continue;
          const normalized = this._normalizeAckStatus(result.status);
          statusById.set(result.id, normalized);
          this.intentStore.updateStatus?.(result.id, normalized, {
            backend: {
              code: result.code || null,
              message: result.message || null,
              updatedAt: Date.now(),
            },
          });
          if (Array.isArray(result.events) && result.events.length) {
            backendEvents.push(...result.events);
          }
        }
        if (
          Array.isArray(backendResult?.events) &&
          backendResult.events.length
        ) {
          backendEvents.push(...backendResult.events);
        }
      } catch (error) {
        for (const entry of stored) {
          statusById.set(entry.id, "failed");
          this.intentStore.updateStatus?.(entry.id, "failed", {
            backend: {
              code: error.code || "BACKEND_REQUEST_FAILED",
              message: error.message || "Backend unavailable",
              updatedAt: Date.now(),
            },
          });
        }
      }
    }

    const ackPayload = intents.map((intent) => {
      const persisted = this.intentStore.get(intent.id);
      return {
        id: intent.id,
        status: statusById.get(intent.id) || persisted?.status || "acked",
        serverTimestamp: persisted?.insertedAt || Date.now(),
        site: persisted?.site || context.site,
        logSeq: persisted?.logSeq || persisted?.insertedAt || Date.now(),
      };
    });
    ws.send(JSON.stringify({ type: "ack", intents: ackPayload }));
    const acknowledged = stored.filter(
      (entry) => (statusById.get(entry.id) || "acked") === "acked",
    );
    this._publishInvalidation(acknowledged, context);
    this._ingestBackendEvents(backendEvents, context);
    for (const entry of acknowledged) {
      this.intentStore.releaseReason?.(entry.id, "pending");
    }
    for (const entry of stored) {
      const finalStatus = statusById.get(entry.id) || entry.status || "acked";
      if (finalStatus !== "acked") {
        this._log(
          "INTENT_REJECTED",
          {
            id: entry.id,
            status: finalStatus,
            site: context.site,
          },
          finalStatus === "failed" ? "warn" : "info",
        );
      } else {
        this._log("INTENT_ACKED", {
          id: entry.id,
          status: finalStatus,
          site: context.site,
        });
      }
    }
  }

  _ingestBackendEvents(events, context) {
    if (!Array.isArray(events) || events.length === 0) return;
    for (const event of events) {
      if (!event || typeof event !== "object") continue;
      const reason = event.reason || "backend-event";
      const timestamp = Number.isFinite(event.timestamp)
        ? event.timestamp
        : Date.now();
      const intents = Array.isArray(event.intents) ? event.intents : [];
      if (event.islandId) {
        const islandPayload = {
          eventId: event.eventId || randomUUID(),
          islandId: event.islandId,
          parameters: event.parameters || {},
          reason,
          site: event.site || context.site,
          cursor: Number.isFinite(event.cursor)
            ? event.cursor
            : this.intentStore.latestCursor?.() || 0,
          timestamp,
          payload: event.payload || {},
        };
        this.eventHub?.publish("island.invalidate", islandPayload);
        this.broadcast("island.invalidate", islandPayload);
      }
      if (intents.length) {
        this.channelRegistry?.broadcast(intents, { reason });
        this.eventHub?.publish("invalidate", {
          reason,
          site: event.site || context.site,
          cursor: Number.isFinite(event.cursor)
            ? event.cursor
            : this.intentStore.latestCursor?.() || 0,
          intents,
        });
      }
    }
  }

  _normalizeAckStatus(status) {
    const normalized = String(status || "").toLowerCase();
    if (["acked", "rejected", "conflict", "failed"].includes(normalized)) {
      return normalized;
    }
    return "failed";
  }

  _validateInboundMessage(ws, message) {
    const validation = {
      "intent.batch": ["optimistic", "INTENT_BATCH"],
      "channel.subscribe": ["channels", "CHANNEL_SUBSCRIBE"],
      "channel.unsubscribe": ["channels", "CHANNEL_UNSUBSCRIBE"],
      "channel.resync": ["channels", "CHANNEL_RESYNC"],
      "channel.command": ["channels", "CHANNEL_COMMAND"],
      ping: ["optimistic", "PING"],
    }[message?.type];
    if (!validation) {
      return true;
    }

    const [group, name] = validation;
    try {
      validateContract(group, name, message);
      return true;
    } catch (error) {
      ws.send(
        JSON.stringify({
          type: "error",
          code: "INVALID_CONTRACT",
          message: error.message,
          details: error.details,
        }),
      );
      return false;
    }
  }

  _publishInvalidation(entries, context) {
    if (!entries.length) return;
    const cursor = this.intentStore.latestCursor?.() || 0;
    this.eventHub?.publish("invalidate", {
      reason: "intent-flush",
      site: context.site,
      cursor,
      intents: entries.map((entry) => ({
        id: entry.id,
        intent: entry.intent,
        payload: entry.payload,
        meta: entry.meta,
        insertedAt: entry.insertedAt,
        logSeq: entry.logSeq || entry.insertedAt,
      })),
    });
    this.channelRegistry?.broadcast(entries, { reason: "intent-flush" });
    this._log("CHANNEL_INVALIDATE", {
      reason: "intent-flush",
      site: context.site,
      intents: entries.map((entry) => entry.id),
      cursor,
    });
  }

  _sendReplay(ws, cursor = 0) {
    if (!this.intentStore) return;
    const backlog = this.intentStore.entriesAfter(cursor, { limit: 500 });
    const nextCursor = backlog.length
      ? backlog[backlog.length - 1].logSeq
      : cursor;
    ws.send(
      JSON.stringify({ type: "replay", intents: backlog, cursor: nextCursor }),
    );
  }

  _isSubprotocolCompatible(clientVersion) {
    const expected = this.subprotocol.split(".").map(Number);
    const actual = clientVersion.split(".").map(Number);
    return expected[0] === actual[0];
  }

  #islandAuthorized(context, islandId) {
    if (!islandId) {
      return false;
    }
    const role = this.#resolveRole(context);
    const requiredRole = this.islandAccess.get(islandId) || "guest";
    return hasRole(role, requiredRole);
  }

  _safeSend(ws, payload, context = null) {
    try {
      const body =
        typeof payload === "string" ? payload : JSON.stringify(payload);
      ws.send(body);
      if (
        this.wsConfig?.maxBufferedBytes &&
        ws.bufferedAmount > this.wsConfig.maxBufferedBytes
      ) {
        this.metrics.backpressureEvents += 1;
        this.#closeSlowConsumer(ws, context, "buffer-overflow");
      }
    } catch (error) {
      console.warn("[SyncGateway] Failed to send payload", error);
    }
  }

  #closeSlowConsumer(ws, context, reason) {
    if (!ws || ws.readyState === ws.CLOSING || ws.readyState === ws.CLOSED) {
      return;
    }
    this.metrics.slowConsumerDrops += 1;
    try {
      ws.close(4002, "SLOW_CONSUMER");
    } catch (error) {
      console.warn("[SyncGateway] Failed to close slow client", error);
    }
    this.logService?.ingestServerEvent?.(
      "WS_SLOW_CONSUMER_DROPPED",
      {
        connectionId: context?.connectionId,
        role: context?.role,
        userId: context?.user?.id,
        reason,
      },
      "warn",
    );
  }

  async _resolveUser(request) {
    if (!this.authService || !request?.headers?.cookie) {
      return null;
    }
    try {
      const cookies = parseCookie(request.headers.cookie || "");
      const token = cookies[this.authCookieName];
      if (!token) return null;
      const payload = await this.authService.verifyToken(token);
      if (!payload) return null;
      return {
        id: payload.sub,
        role: payload.role || "user",
      };
    } catch (error) {
      console.warn("[SyncGateway] Failed to verify auth token:", error);
      return null;
    }
  }

  _getIp(request) {
    return (
      request.headers["x-forwarded-for"]?.split(",")[0] ||
      request.socket?.remoteAddress ||
      ""
    ).trim();
  }

  #computeLatency(payload) {
    if (!payload || !Number.isFinite(payload.timestamp)) {
      return null;
    }
    return Math.max(0, Date.now() - payload.timestamp);
  }

  #recordLatency(latency) {
    const bucket = this.metrics.latency;
    bucket.count += 1;
    bucket.sum += latency;
    bucket.max = Math.max(bucket.max, latency);
    bucket.avg = bucket.sum / bucket.count;
    this.monitor?.recordInvalidationLatency("ws", latency);
  }

  _consumeChannelRate(context) {
    const limits = this.channelRateLimit;
    if (!limits || !limits.max || limits.max <= 0) {
      return { allowed: true };
    }
    const now = Date.now();
    const windowMs = limits.windowMs || 10 * 1000;
    const key = `${limits.name || "channel-sub"}:${this._channelRateKey(context)}`;
    const bucket = this.channelBuckets.get(key) || {
      count: 0,
      expiresAt: now + windowMs,
    };
    if (bucket.expiresAt < now) {
      bucket.count = 0;
      bucket.expiresAt = now + windowMs;
    }
    bucket.count += 1;
    this.channelBuckets.set(key, bucket);
    if (bucket.count > limits.max) {
      return { allowed: false, retryAfterMs: bucket.expiresAt - now };
    }
    return { allowed: true };
  }

  _channelRateKey(context) {
    const userId = context.user?.id || "anonymous";
    const role = this.#resolveRole(context);
    const site = context.site || "default";
    const ip = context.ip || "unknown";
    return `${userId}:${role}:${site}:${ip}`;
  }

  #resolveRole(context) {
    const candidate = context?.user?.role || context?.role || "guest";
    const allowed = new Set(["guest", "user", "staff", "admin", "system"]);
    return allowed.has(candidate) ? candidate : "guest";
  }

  _log(type, data, level = "info") {
    this.logService?.ingestServerEvent(type, data, level);
  }

  getMetrics() {
    return {
      ...this.metrics,
      broadcastsByType: { ...this.metrics.broadcastsByType },
    };
  }
}
