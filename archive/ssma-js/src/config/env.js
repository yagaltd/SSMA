import path from "node:path";
import { fileURLToPath } from "node:url";
import dotenv from "dotenv";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

dotenv.config({ path: path.join(__dirname, "../../.env") });

const toNumber = (value, fallback) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
};

export function loadConfig() {
  const canonicalSubprotocol = process.env.SSMA_PROTOCOL_SUBPROTOCOL;
  const legacySubprotocol = process.env.SSMA_OPTIMISTIC_SUBPROTOCOL;
  const resolvedSubprotocol = canonicalSubprotocol || legacySubprotocol || "1.0.0";
  if (!canonicalSubprotocol && legacySubprotocol) {
    console.warn(
      "[config] SSMA_OPTIMISTIC_SUBPROTOCOL is deprecated; use SSMA_PROTOCOL_SUBPROTOCOL",
    );
  }

  return {
    server: {
      port: toNumber(process.env.SSMA_PORT, 5050),
    },
    security: {
      jwtSecret: process.env.SSMA_JWT_SECRET || "change-me-in-production",
      jwtIssuer: process.env.SSMA_JWT_ISSUER || "ssma-auth-service",
      jwtAudience: process.env.SSMA_JWT_AUDIENCE || "csma-clients",
      accessTokenTtl: toNumber(process.env.SSMA_ACCESS_TTL_MS, 15 * 60 * 1000),
      refreshTokenTtl: toNumber(
        process.env.SSMA_REFRESH_TTL_MS,
        7 * 24 * 60 * 60 * 1000,
      ),
      hmacSecret: process.env.SSMA_HMAC_SECRET || "replace-hmac-secret",
      hmacTTL: toNumber(process.env.SSMA_HMAC_TTL_MS, 5 * 60 * 1000),
    },
    cors: {
      origins: (process.env.SSMA_ALLOWED_ORIGINS || "*")
        .split(",")
        .map((origin) => origin.trim()),
    },
    rateLimit: {
      windowMs: toNumber(process.env.SSMA_RATE_WINDOW_MS, 60 * 1000),
      max: toNumber(process.env.SSMA_RATE_MAX, 120),
    },
    logs: {
      exporter: (process.env.SSMA_LOG_EXPORTER || "console").toLowerCase(),
      bufferSize: toNumber(process.env.SSMA_LOG_BUFFER_SIZE, 2000),
      filePath: process.env.SSMA_LOG_FILE
        ? path.resolve(process.cwd(), process.env.SSMA_LOG_FILE)
        : path.resolve(__dirname, "../../logs/ssma.log"),
      maxBatchSize: toNumber(process.env.SSMA_LOG_MAX_BATCH, 200),
    },
    optimistic: {
      adapter: (process.env.SSMA_OPTIMISTIC_ADAPTER || "file").toLowerCase(),
      storePath: process.env.SSMA_OPTIMISTIC_STORE
        ? path.resolve(process.cwd(), process.env.SSMA_OPTIMISTIC_STORE)
        : path.resolve(process.cwd(), "data/optimistic-intents.json"),
      replayWindowMs: toNumber(
        process.env.SSMA_OPTIMISTIC_REPLAY_MS,
        5 * 60 * 1000,
      ),
      maxEntries: toNumber(process.env.SSMA_OPTIMISTIC_MAX_ENTRIES, 5000),
      subprotocol: resolvedSubprotocol,
      rateLimit: {
        rework: {
          windowMs: toNumber(
            process.env.SSMA_OPTIMISTIC_REWORK_WINDOW_MS,
            60 * 1000,
          ),
          max: toNumber(process.env.SSMA_OPTIMISTIC_REWORK_MAX, 20),
        },
        channel: {
          windowMs: toNumber(
            process.env.SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS,
            10 * 1000,
          ),
          max: toNumber(process.env.SSMA_OPTIMISTIC_CHANNEL_MAX, 8),
        },
      },
      requireAuthForWrites:
        process.env.SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES === "true",
      transport: {
        sse: {
          maxQueueBytes: toNumber(
            process.env.SSMA_SSE_MAX_QUEUE_BYTES,
            64 * 1024,
          ),
          drainTimeoutMs: toNumber(process.env.SSMA_SSE_DRAIN_TIMEOUT_MS, 5000),
          retryMs: toNumber(process.env.SSMA_SSE_RETRY_MS, 2500),
        },
        ws: {
          maxBufferedBytes: toNumber(
            process.env.SSMA_WS_MAX_BUFFERED_BYTES,
            256 * 1024,
          ),
          slowConsumerCloseMs: toNumber(
            process.env.SSMA_WS_SLOW_CONSUMER_CLOSE_MS,
            10 * 1000,
          ),
        },
        islandAccess: {
          "product-inventory": "guest",
          "product-reviews": "user",
          "blog-comments": "user",
          "hydration-test": "guest",
          "ops.dashboard": "staff",
        },
      },
    },
    backend: {
      url: process.env.SSMA_BACKEND_URL || "",
      timeoutMs: toNumber(process.env.SSMA_BACKEND_TIMEOUT_MS, 5000),
      internalToken: process.env.SSMA_BACKEND_INTERNAL_TOKEN || "",
      emitEvents: process.env.SSMA_BACKEND_EMIT_EVENTS !== "false",
    },
    auth: {
      userStorePath: process.env.SSMA_USER_STORE
        ? path.resolve(process.cwd(), process.env.SSMA_USER_STORE)
        : path.resolve(process.cwd(), "data/users.json"),
      jwtSecret: process.env.SSMA_AUTH_JWT_SECRET || "change-me-in-production",
      jwtExpiresIn: process.env.SSMA_AUTH_JWT_EXPIRES_IN || "15m",
      cookieName: process.env.SSMA_AUTH_COOKIE || "ssma_session",
      cookieSecure: process.env.SSMA_AUTH_COOKIE_SECURE
        ? process.env.SSMA_AUTH_COOKIE_SECURE !== "false"
        : process.env.NODE_ENV === "production",
    },
    monitoring: {
      backlogThreshold: toNumber(
        process.env.SSMA_MONITOR_BACKLOG_THRESHOLD,
        1000,
      ),
      invalidationLatencyBudgetMs: toNumber(
        process.env.SSMA_MONITOR_INVALIDATION_BUDGET_MS,
        5000,
      ),
    },
    features: {
      staticRender: {
        enabled: process.env.SSMA_STATIC_RENDER_ENABLED !== "false",
        disabledIslands: (process.env.SSMA_STATIC_RENDER_DISABLED_ISLANDS || "")
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
      },
    },
  };
}
