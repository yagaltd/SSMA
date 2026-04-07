export function registerBackendRoutes(kernel, { eventHub, syncGateway, channelRegistry, logService, config }) {
  kernel.route('POST', '/internal/backend/events', (ctx) => {
    const configuredToken = config?.backend?.internalToken || '';
    const requestToken = ctx.headers['x-ssma-backend-token'];
    if (configuredToken && requestToken !== configuredToken) {
      ctx.json(401, { error: 'UNAUTHORIZED_BACKEND_EVENT_SOURCE' });
      return;
    }

    const payload = ctx.body || {};
    const events = Array.isArray(payload.events)
      ? payload.events
      : payload.event
        ? [payload.event]
        : [];

    let processed = 0;
    for (const event of events) {
      if (!event || typeof event !== 'object') continue;
      processed += 1;
      const reason = event.reason || 'backend-event';
      const timestamp = Number.isFinite(event.timestamp) ? event.timestamp : Date.now();

      if (event.islandId) {
        const islandPayload = {
          eventId: event.eventId || `backend-${timestamp}-${processed}`,
          islandId: event.islandId,
          parameters: event.parameters || {},
          reason,
          site: event.site || 'default',
          cursor: Number.isFinite(event.cursor) ? event.cursor : timestamp,
          timestamp,
          payload: event.payload || {}
        };
        eventHub?.publish('island.invalidate', islandPayload);
        syncGateway?.broadcast('island.invalidate', islandPayload);
      }

      const intents = Array.isArray(event.intents) ? event.intents : [];
      if (intents.length) {
        channelRegistry?.broadcast(intents, { reason });
        eventHub?.publish('invalidate', {
          reason,
          site: event.site || 'default',
          cursor: Number.isFinite(event.cursor) ? event.cursor : timestamp,
          intents
        });
      }

      logService?.ingestServerEvent?.('BACKEND_EVENT_INGESTED', {
        type: event.type || 'unknown',
        reason,
        site: event.site || 'default',
        islandId: event.islandId || null,
        intents: intents.map((intent) => intent.id)
      });
    }

    ctx.json(202, {
      status: 'accepted',
      processed
    });
  });
}
