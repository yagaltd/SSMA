import { createNodeKernel } from "./runtime/kernel/nodeKernel.js";
import { requestLogger } from "./middleware/requestLogger.js";
import { corsMiddleware } from "./middleware/cors.js";
import { rateLimiter } from "./middleware/rateLimiter.js";
import { registerLogRoutes } from "./routes/logRoutes.js";
import { LogService } from "./services/log-accumulator/LogService.js";
import { registerAuthRoutes } from "./routes/authRoutes.js";
import { registerOptimisticRoutes } from "./routes/optimisticRoutes.js";
import { registerIslandRoutes } from "./routes/islandRoutes.js";
import { registerBackendRoutes } from "./routes/backendRoutes.js";
import { IntentStore } from "./services/optimistic/IntentStore.js";
import { OptimisticEventHub } from "./services/optimistic/OptimisticEventHub.js";
import { ChannelRegistry } from "./services/optimistic/ChannelRegistry.js";
import { SyncGateway } from "./services/optimistic/SyncGateway.js";
import { UserStore } from "./services/auth/UserStore.js";
import { AuthService } from "./services/auth/AuthService.js";
import { authMiddleware } from "./middleware/authMiddleware.js";
import { hasRole } from "./utils/rbac.js";
import { IslandInvalidationService } from "./services/island-invalidations/IslandInvalidationService.js";
import { HybridMonitor } from "./services/monitoring/HybridMonitor.js";
import { BackendHttpClient } from "./backend/BackendHttpClient.js";

function attachGracefulClose(server, syncGateway) {
  const originalClose = server.close.bind(server);
  let closePromise = null;

  server.closeGracefully = async () => {
    if (!closePromise) {
      closePromise = Promise.resolve(syncGateway?.destroy?.()).finally(
        () =>
          new Promise((resolve) => {
            originalClose(() => resolve());
          }),
      );
    }
    return closePromise;
  };

  server.close = (callback) => {
    server
      .closeGracefully()
      .then(() => callback?.())
      .catch((error) => {
        process.nextTick(() => {
          throw error;
        });
      });
    return server;
  };
}

export function createServer(config) {
  const kernel = createNodeKernel(config);
  const logService = new LogService({
    bufferSize: config.logs.bufferSize,
    maxBatchSize: config.logs.maxBatchSize,
    exporter: config.logs.exporter,
    filePath: config.logs.filePath,
  });

  kernel.use(requestLogger());
  kernel.use(corsMiddleware({ origins: config.cors.origins }));
  kernel.use(
    rateLimiter({
      name: "global",
      windowMs: config.rateLimit.windowMs,
      max: config.rateLimit.max,
      onLimit: async (ctx, bucket) => {
        await logService.ingestServerEvent(
          "RATE_LIMIT_HIT",
          {
            ip: ctx.ip,
            path: ctx.path,
            method: ctx.method,
            retryAfterMs: bucket.expiresAt - Date.now(),
            limiter: bucket.name || "global",
          },
          "warn",
        );
      },
    }),
  );

  const hybridMonitor = new HybridMonitor({
    logService,
    backlogThreshold: config.monitoring?.backlogThreshold,
    invalidationLatencyBudgetMs: config.monitoring?.invalidationLatencyBudgetMs,
  });

  const userStore = new UserStore({ filePath: config.auth.userStorePath });
  const authService = new AuthService({
    userStore,
    jwtSecret: config.auth.jwtSecret,
    jwtExpiresIn: config.auth.jwtExpiresIn,
  });

  kernel.use(authMiddleware(authService, config));

  const reworkRateLimit = config.optimistic.rateLimit?.rework || {
    windowMs: 60 * 1000,
    max: 20,
  };
  kernel.use(
    rateLimiter({
      name: "optimistic-rework",
      windowMs: reworkRateLimit.windowMs,
      max: reworkRateLimit.max,
      shouldApply: (ctx) =>
        ctx.method === "POST" &&
        (ctx.path === "/optimistic/rework" || ctx.path === "/optimistic/undo"),
      keyResolver: (ctx) => {
        const userId = ctx.state.user?.id || ctx.ip || "anonymous";
        const role = ctx.state.user?.role || "guest";
        return `${userId}:${role}`;
      },
      onLimit: async (ctx, bucket) => {
        await logService.ingestServerEvent(
          "OPTIMISTIC_REWORK_RATE_LIMIT",
          {
            path: ctx.path,
            userId: ctx.state.user?.id,
            role: ctx.state.user?.role,
            ip: ctx.ip,
            retryAfterMs: bucket.retryAfterMs,
            windowMs: bucket.windowMs,
            max: bucket.max,
          },
          "warn",
        );
      },
    }),
  );

  kernel.route("GET", "/health", (ctx) => {
    const logHealth = logService.health();
    ctx.json(200, {
      status: logHealth.status === "degraded" ? "degraded" : "ok",
      service: "ssma",
      timestamp: Date.now(),
      logPipeline: logHealth,
    });
  });

  const intentStore = new IntentStore({
    adapter: { type: config.optimistic.adapter },
    filePath: config.optimistic.storePath,
    maxEntries: config.optimistic.maxEntries,
    replayWindowMs: config.optimistic.replayWindowMs,
    monitor: hybridMonitor,
  });
  config.intentStore = intentStore;
  const optimisticHub = new OptimisticEventHub({
    transport: config.optimistic.transport,
    logService,
    monitor: hybridMonitor,
  });
  const channelRegistry = new ChannelRegistry({
    intentStore,
    replayWindowMs: config.optimistic.replayWindowMs,
  });
  const backendClient = new BackendHttpClient({
    baseUrl: config.backend?.url,
    timeoutMs: config.backend?.timeoutMs,
  });

  channelRegistry.registerChannel("global", {
    load: ({ intentStore: store, cursor = 0, limit = 200 }) =>
      store.entriesAfter(cursor, { limit, channels: ["global"] }),
  });

  channelRegistry.registerChannel("ops.audit", {
    access: (_, ctx) => hasRole(ctx.connection?.user?.role, "staff"),
    load: ({ intentStore: store, cursor = 0, limit = 200 }) =>
      store.entriesAfter(cursor, { limit, channels: ["ops.audit"] }),
  });

  const syncGateway = new SyncGateway({
    server: kernel.server,
    intentStore,
    eventHub: optimisticHub,
    channelRegistry,
    authService,
    authCookieName: config.auth.cookieName,
    logService,
    channelRateLimit: config.optimistic.rateLimit?.channel,
    allowedOrigins: config.cors.origins,
    replayWindowMs: config.optimistic.replayWindowMs,
    subprotocol: config.optimistic.subprotocol || "1.0.0",
    transport: config.optimistic.transport,
    monitor: hybridMonitor,
    backendClient,
    requireAuthForWrites: config.optimistic.requireAuthForWrites,
  });

  syncGateway.start();
  attachGracefulClose(kernel.server, syncGateway);

  const islandInvalidationService = new IslandInvalidationService({
    eventHub: optimisticHub,
    syncGateway,
    logService,
    featureFlags: config.features?.staticRender,
  });

  config.intentStore = intentStore;
  config.channelRegistry = channelRegistry;
  config.islandInvalidationService = islandInvalidationService;

  registerLogRoutes(kernel, logService, config);
  registerAuthRoutes(kernel, authService, config);
  registerOptimisticRoutes(
    kernel,
    optimisticHub,
    channelRegistry,
    logService,
    intentStore,
    syncGateway,
    hybridMonitor,
  );
  registerBackendRoutes(kernel, {
    eventHub: optimisticHub,
    syncGateway,
    channelRegistry,
    logService,
    config,
  });
  registerIslandRoutes(kernel, islandInvalidationService);

  return kernel.server;
}
