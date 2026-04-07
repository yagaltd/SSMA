# Admin APIs

- `GET /admin/optimistic/channels`
- `GET /admin/optimistic/intents`
- `GET /optimistic/metrics`

## `GET /optimistic/metrics`

Returns operational counters for the Rust optimistic gateway:

- `active.ws`, `active.sse`
- `totals.wsConnections`, `totals.sseConnections`
- `totals.broadcasts`
- `totals.rateLimitHits`
- `totals.sseClientDropped`
- `store.cursor`, `store.replayDepth`
- `serverEvents` map (for example: `INTENT_ACKED`, `CHANNEL_SUBSCRIBE`, `SSE_CLIENT_DROPPED`)
