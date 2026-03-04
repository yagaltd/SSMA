import { hasRole } from '../../utils/rbac.js';

export class OptimisticEventHub {
  constructor({ transport = {}, logService, monitor } = {}) {
    this.sequence = 0;
    this.clients = new Map();
    this.logService = logService;
    this.monitor = monitor || null;
    this.config = {
      maxQueueBytes: transport?.sse?.maxQueueBytes ?? 64 * 1024,
      drainTimeoutMs: transport?.sse?.drainTimeoutMs ?? 5000,
      retryMs: transport?.sse?.retryMs ?? 2500
    };
    this.accessMatrix = new Map(Object.entries(transport?.islandAccess || {}));
    this.metrics = {
      streamsOpened: 0,
      streamsClosed: 0,
      streamsActive: 0,
      eventsTotal: 0,
      eventsByType: {},
      lastIslandInvalidateAt: null,
      backpressureEvents: 0,
      slowClientDrops: 0,
      unauthorizedFiltered: 0,
      latency: { count: 0, sum: 0, max: 0, avg: 0 }
    };
  }

  addClient(ctx, options = {}) {
    const res = ctx.res;
    const origin = ctx.headers.origin || '*';
    if (!res) {
      throw new Error('[OptimisticEventHub] response is required');
    }
    this.sequence += 1;
    const clientId = `sse-${this.sequence}`;
    const allowedIslands = Array.isArray(options.islands)
      ? options.islands
      : this.#parseIslands(ctx.query?.islands);
    const client = {
      id: clientId,
      res,
      role: ctx.state?.user?.role || 'guest',
      userId: ctx.state?.user?.id || 'anonymous',
      allowedIslands,
      queue: [],
      queueBytes: 0,
      paused: false,
      drainTimer: null,
      createdAt: Date.now(),
      lastEventAt: null,
      ip: ctx.ip,
      userAgent: ctx.headers['user-agent']
    };
    this.clients.set(clientId, client);
    this.metrics.streamsOpened += 1;
    this.metrics.streamsActive = this.clients.size;

    res.statusCode = 200;
    res.setHeader('Content-Type', 'text/event-stream');
    res.setHeader('Cache-Control', 'no-cache');
    res.setHeader('Connection', 'keep-alive');
    res.setHeader('Access-Control-Allow-Origin', origin);
    res.setHeader('Access-Control-Allow-Credentials', 'true');
    res.setHeader('Access-Control-Expose-Headers', 'Content-Type');
    res.setHeader('Vary', 'Origin');
    res.flushHeaders?.();
    res.write(`retry: ${this.config.retryMs}\n`);
    res.write(`event: ready\ndata: {"clientId":"${clientId}"}\n\n`);

    const cleanup = () => this.#removeClient(clientId, 'disconnect');
    res.on('close', cleanup);
    res.on('error', cleanup);

    return clientId;
  }

  publish(event, payload) {
    if (!event) return;
    this.metrics.eventsTotal += 1;
    this.metrics.eventsByType[event] = (this.metrics.eventsByType[event] || 0) + 1;
    if (event === 'island.invalidate') {
      this.metrics.lastIslandInvalidateAt = Date.now();
      console.log(
        `[OptimisticEventHub] SSE forwarded island.invalidate #${this.metrics.eventsByType[event]}`
      );
      const latency = this.#computeLatency(payload);
      if (latency !== null) {
        this.#recordLatency(latency);
      }
    }
    const frame = this.#formatEvent(event, payload);
    for (const [clientId, client] of this.clients.entries()) {
      if (!this.#isAuthorized(client, event, payload)) {
        this.metrics.unauthorizedFiltered += 1;
        continue;
      }
      this.#dispatch(clientId, client, frame);
    }
  }

  getMetrics() {
    return {
      ...this.metrics,
      eventsByType: { ...this.metrics.eventsByType }
    };
  }

  resetMetrics() {
    this.metrics = {
      streamsOpened: 0,
      streamsClosed: 0,
      streamsActive: this.clients.size,
      eventsTotal: 0,
      eventsByType: {},
      lastIslandInvalidateAt: null,
      backpressureEvents: 0,
      slowClientDrops: 0,
      unauthorizedFiltered: 0,
      latency: { count: 0, sum: 0, max: 0, avg: 0 }
    };
  }

  #dispatch(clientId, client, frame) {
    if (client.paused) {
      this.#enqueue(clientId, client, frame);
      return;
    }
    const ok = client.res.write(frame);
    client.lastEventAt = Date.now();
    if (!ok) {
      this.metrics.backpressureEvents += 1;
      client.paused = true;
      this.#enqueue(clientId, client, '');
      client.res.once('drain', () => this.#drain(clientId));
      client.drainTimer = setTimeout(() => {
        this.#dropClient(clientId, 'backpressure-timeout');
      }, this.config.drainTimeoutMs);
    }
  }

  #enqueue(clientId, client, frame) {
    const size = Buffer.byteLength(frame);
    client.queue.push(frame);
    client.queueBytes += size;
    if (client.queueBytes > this.config.maxQueueBytes) {
      this.#dropClient(clientId, 'backpressure-overflow');
    }
  }

  #drain(clientId) {
    const client = this.clients.get(clientId);
    if (!client) return;
    clearTimeout(client.drainTimer);
    client.drainTimer = null;
    client.paused = false;
    while (client.queue.length && !client.paused) {
      const frame = client.queue.shift();
      client.queueBytes -= Buffer.byteLength(frame);
      this.#dispatch(clientId, client, frame);
    }
  }

  #formatEvent(event, payload) {
    const body = typeof payload === 'string' ? payload : JSON.stringify(payload ?? {});
    return `event: ${event}\ndata: ${body}\n\n`;
  }

  #isAuthorized(client, event, payload) {
    if (event !== 'island.invalidate') {
      return true;
    }
    const islandId = payload?.islandId;
    if (!islandId) {
      return false;
    }
    if (Array.isArray(client.allowedIslands) && client.allowedIslands.length) {
      if (!client.allowedIslands.includes(islandId)) {
        return false;
      }
    }
    const requiredRole = this.accessMatrix.get(islandId) || 'guest';
    return hasRole(client.role, requiredRole);
  }

  #dropClient(clientId, reason) {
    const client = this.clients.get(clientId);
    if (!client) return;
    this.metrics.slowClientDrops += 1;
    this.#removeClient(clientId, reason);
    this.logService?.ingestServerEvent?.('SSE_CLIENT_DROPPED', {
      clientId,
      reason,
      role: client.role,
      userId: client.userId,
      ip: client.ip
    }, 'warn');
  }

  #removeClient(clientId, reason) {
    const client = this.clients.get(clientId);
    if (!client) return;
    clearTimeout(client.drainTimer);
    try {
      if (reason === 'backpressure-overflow') {
        client.res.write('event: retry\ndata: {"reason":"backpressure"}\n\n');
      }
      client.res.end();
    } catch (error) {
      // ignore
    }
    this.clients.delete(clientId);
    this.metrics.streamsClosed += 1;
    this.metrics.streamsActive = this.clients.size;
  }

  #parseIslands(raw) {
    if (!raw) return null;
    if (Array.isArray(raw)) {
      return raw.flatMap((value) => this.#parseIslands(value) || []);
    }
    if (typeof raw !== 'string') return null;
    return raw
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean);
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
    this.monitor?.recordInvalidationLatency('sse', latency);
  }
}
