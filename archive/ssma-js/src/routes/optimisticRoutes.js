import { hasRole } from '../utils/rbac.js';

export function registerOptimisticRoutes(kernel, eventHub, channelRegistry, logService, intentStore, syncGateway = null, monitor = null) {
  const ensureRole = (ctx, role = 'staff') => {
    if (!hasRole(ctx.state.user?.role, role)) {
      ctx.json(403, { error: 'FORBIDDEN', requiredRole: role });
      return false;
    }
    return true;
  };
  kernel.route('GET', '/optimistic/events', (ctx) => {
    const islands = typeof ctx.query?.islands === 'string'
      ? ctx.query.islands.split(',').map((value) => value.trim()).filter(Boolean)
      : undefined;
    const clientId = eventHub.addClient(ctx, { islands });
    ctx.responded = true;
    console.log(`[Optimistic] SSE client connected: ${clientId}`);

    const store = ctx.config.intentStore;
    if (store) {
      const snapshot = store.entriesAfter(0, { limit: 500 });
      if (snapshot.length) {
        const cursor = snapshot[snapshot.length - 1].logSeq || snapshot[snapshot.length - 1].insertedAt;
        ctx.res.write(`event: replay\ndata: ${JSON.stringify({ intents: snapshot, cursor })}\n\n`);
      }
    }
  });

  kernel.route('GET', '/optimistic/pending', (ctx) => {
    const store = ctx.config.intentStore;
    if (!store) {
      ctx.json(200, { pending: [] });
      return;
    }
    const since = Number(ctx.query.since || 0);
    const entries = Number.isFinite(since) && since > 0 ? store.entriesSince(since) : store.entries();
    ctx.json(200, { pending: entries });
  });

  kernel.route('POST', '/optimistic/rework', (ctx) => {
    if (!ensureRole(ctx, 'staff')) return;
    const store = ctx.config.intentStore;
    const { id } = ctx.body || {};
    if (!store) {
      ctx.json(500, { error: 'STORE_UNAVAILABLE' });
      return;
    }
    const entry = store.get(id);
    if (!entry) {
      ctx.json(404, { error: 'NOT_FOUND' });
      return;
    }
    if (!entry.meta?.undo) {
      ctx.json(400, { error: 'UNDO_NOT_AVAILABLE' });
      return;
    }
    store.addReason?.(id, 'rework');
    const payload = {
      reason: ctx.body?.reason || 'manual-rework',
      intents: [
        {
          id: entry.id,
          intent: entry.intent,
          undo: entry.meta.undo,
          payload: entry.payload,
          channels: entry.meta.channels || ['global'],
          site: entry.site
        }
      ]
    };
    eventHub.publish('rework', payload);
    logService?.ingestServerEvent('OPTIMISTIC_REWORK_ENQUEUED', {
      entryId: entry.id,
      reason: ctx.body?.reason || 'manual-rework',
      channels: entry.meta?.channels || [],
      userId: ctx.state.user?.id,
      role: ctx.state.user?.role
    });
    channelRegistry?.broadcast([entry], { reason: 'intent-rework' });
    ctx.json(202, { status: 'scheduled', id: entry.id });
  });

  kernel.route('POST', '/optimistic/undo', (ctx) => {
    if (!ensureRole(ctx, 'user')) return;
    const store = ctx.config.intentStore;
    if (!store) {
      ctx.json(500, { error: 'STORE_UNAVAILABLE' });
      return;
    }
    const { id, intent, payload } = ctx.body || {};
    if (!id || !intent) {
      ctx.json(400, { error: 'INVALID_PAYLOAD' });
      return;
    }
    const entry = store.get(id);
    if (!entry || !entry.meta?.undo) {
      ctx.json(404, { error: 'NOT_FOUND' });
      return;
    }
    const expected = entry.meta.undo;
    if (expected.intent !== intent || !_deepEqual(expected.payload, payload)) {
      ctx.json(409, { error: 'UNDO_MISMATCH' });
      return;
    }

    store.releaseReason(id, 'replay');
    store.releaseReason(id, 'rework');
    for (const channel of entry.meta.channels || []) {
      store.releaseReason(id, `channel:${channel}`);
    }

    eventHub.publish('undo', {
      id,
      intent: entry.intent,
      reason: ctx.body?.reason || 'client-undo'
    });
    logService?.ingestServerEvent('OPTIMISTIC_UNDO_CONFIRMED', {
      entryId: id,
      userId: ctx.state.user?.id,
      role: ctx.state.user?.role,
      channels: entry.meta?.channels || []
    });
    channelRegistry?.broadcast([entry], { reason: 'intent-undo' });
    ctx.json(200, { status: 'reverted', id });
  });

  kernel.route('GET', '/admin/optimistic/channels', (ctx) => {
    if (!ensureRole(ctx, 'staff')) return;
    const snapshot = channelRegistry?.listSubscriptions?.() || [];
    const summary = new Map();
    for (const sub of snapshot) {
      if (!summary.has(sub.channel)) {
        summary.set(sub.channel, {
          channel: sub.channel,
          total: 0,
          subscribers: []
        });
      }
      const channelInfo = summary.get(sub.channel);
      channelInfo.total += 1;
      channelInfo.subscribers.push({
        connectionId: sub.connectionId,
        params: sub.params,
        subscribedAt: sub.subscribedAt,
        connectionRole: sub.connectionRole,
        site: sub.site,
        user: sub.user ? { id: sub.user.id, role: sub.user.role } : null
      });
    }
    ctx.json(200, {
      updatedAt: Date.now(),
      totalSubscriptions: snapshot.length,
      channels: Array.from(summary.values())
    });
  });

  kernel.route('GET', '/admin/optimistic/intents', (ctx) => {
    if (!ensureRole(ctx, 'staff')) return;
    const store = intentStore || ctx.config?.intentStore;
    const limit = Math.max(1, Math.min(Number(ctx.query.limit) || 100, 500));
    const reasonFilter = ctx.query.reason;
    const entries = store?.entries ? store.entries() : [];
    const filtered = reasonFilter
      ? entries.filter((entry) => (entry.meta?.reasons || []).includes(reasonFilter))
      : entries;
    const summary = new Map();
    for (const entry of filtered) {
      for (const reason of entry.meta?.reasons || []) {
        summary.set(reason, (summary.get(reason) || 0) + 1);
      }
    }
    ctx.json(200, {
      updatedAt: Date.now(),
      pending: filtered.slice(0, limit).map((entry) => ({
        id: entry.id,
        intent: entry.intent,
        channels: entry.meta?.channels || ['global'],
        reasons: entry.meta?.reasons || [],
        site: entry.site,
        status: entry.status,
        connectionId: entry.connectionId,
        insertedAt: entry.insertedAt
      })),
      reasonSummary: Array.from(summary.entries()).map(([reason, count]) => ({ reason, count })),
      total: filtered.length
    });
  });

  kernel.route('GET', '/optimistic/metrics', (ctx) => {
    const sseMetrics = eventHub?.getMetrics?.() || null;
    const wsMetrics = syncGateway?.getMetrics?.() || null;
    const hybridSnapshot = monitor?.snapshot?.() || null;
    const backlog = hybridSnapshot?.backlogDepth ?? (intentStore?.entries ? intentStore.entries().length : null);
    const channelSubscriptions = channelRegistry?.listSubscriptions?.()?.length || 0;
    ctx.json(200, {
      updatedAt: Date.now(),
      sse: sseMetrics,
      ws: wsMetrics,
      backlog,
      channelSubscriptions,
      hybrid: hybridSnapshot
    });
  });
}

function _deepEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}
