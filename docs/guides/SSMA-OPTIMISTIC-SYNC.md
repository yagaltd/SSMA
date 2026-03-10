# SSMA Optimistic Sync Gateway

This guide documents how the SSMA backend ingests optimistic actions from CSMA clients, validates them, and streams acknowledgements/invalidation events back to the fleet. It complements the CSMA-side `optimistic-sync` module docs.

## Runtime Components

| Component | Path | Responsibility |
|-----------|------|----------------|
| `SyncGateway` | `src/services/optimistic/SyncGateway.js` | Accepts WebSocket upgrades on `/optimistic/ws`, validates subprotocol, receives intent batches from the leader tab, and emits ACK/REPLAY/INVALIDATION messages. |
| `IntentStore` | `src/services/optimistic/IntentStore.js` | Durable persistence layer backed by pluggable adapters (JSON file default, optional SQLite) with Lamport clocks, reason tracking, channel metadata, and replay trimming. |
| `OptimisticEventHub` | `src/services/optimistic/OptimisticEventHub.js` | SSE broadcaster for `/optimistic/events`, pushing real-time invalidations + initial replay snapshots to follower tabs. |
| `optimisticRoutes` | `src/routes/optimisticRoutes.js` | HTTP routes for SSE clients and debugging (`GET /optimistic/pending`). |

## Transport Protocol

### Connection handshake

CSMA leaders open a WebSocket to `/optimistic/ws` with query params:

```
/optimistic/ws?role=leader&site=<siteId>&subprotocol=<semver>
```

Gateway checks `subprotocol` (major version must match `config.optimistic.subprotocol`) and immediately sends:

```json
{
  "type": "hello",
  "role": "leader",
  "authRole": "user",
  "connectionId": "uuid",
  "serverTime": 1734370000,
  "subprotocol": "1.0.0"
}
```

Leaders can then send `intent.batch` payloads; followers ignore writes and rely on SSE or channel fanout only. If no valid session cookie is present, the connection still succeeds but `authRole` falls back to `guest`.

### Intent schema & validation

- All batches are validated against `docs/contracts/optimistic.json#INTENT_BATCH` via AJV before persistence.
- Each intent requires `{ id, intent, payload, meta.clock, meta.reasons?, meta.channels? }`.
- Gateway rejects invalid payloads with `{ type: "error", code: "INVALID_CONTRACT" }`.

### ACK + invalidations

When a batch passes validation:

1. `IntentStore.append` persists every entry, tagging it with `reasons = ['pending','replay','channel:<id>…]` and site/connection metadata.
2. Gateway responds with `{ type: 'ack', intents: [{ id, status, serverTimestamp, site }] }`.
3. `OptimisticEventHub` publishes `event: invalidate` SSE messages so follower tabs refresh.
4. Pending reasons are cleared (`releaseReason(id, 'pending')`); replay reasons expire automatically past `config.optimistic.replayWindowMs`.

Channel subscriptions use a parallel path:

1. `channel.subscribe` returns `channel.ack` and then `channel.snapshot`.
2. `channel.snapshot`, `channel.replay`, and `channel.invalidate` preserve the original subscription `params`.
3. Each invalidation frame targets one `channel`; fanout no longer relies on a `channels[]` aggregate payload.

### Replay

- On websocket connect **and** SSE connect (`/optimistic/events`), SSMA sends `{ type: 'replay', intents: [...] }` containing all entries newer than `replayWindowMs` (default 5 minutes).
- CSMA’s `ChannelManager` routes these events to channel subscribers so local caches hydrate without full refetch.

## Authentication & RBAC

SSMA uses one session-token contract everywhere:

- `AuthService` issues `argon2id`-backed credentials with `jose` JWTs, exposed via `/auth/register`, `/auth/login`, `/auth/logout`, `/auth/me`.
- `authMiddleware` parses the HTTP cookie (`SSMA_AUTH_COOKIE`, default `ssma_session`) and injects `ctx.state.user` for every route.
- `SyncGateway` inspects the same cookie during WebSocket upgrade, verifies the token, and attaches `{ user, role, site }` context to each connection so channel access hooks have full RBAC data.
- `ChannelRegistry` delegates every subscription through `access(params, { connection })` handlers, so sensitive channels (e.g. `ops.audit`) can require `staff` or `system` roles before replay snapshots are sent.
- HTTP routes (`/optimistic/rework`, `/optimistic/undo`, `/admin/optimistic/*`) gate access through role helpers, ensuring only privileged operators can rework or inspect intents.
- **Guests vs. WS** – unauthenticated clients are allowed to connect and operate as `guest`. Protected channels and island invalidations still enforce RBAC, and writes can still be blocked when `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES=true`.
- **SSE and RBAC** – `/optimistic/events` stays open to guests, but island invalidations are filtered by role and optional `?islands=a,b` scoping before delivery. SSE never delivers ACKs, so anonymous users will still see pending intents if they try to publish while writes require auth.

## Rate limiting & transport hardening

- **HTTP rework/undo**: the global middleware now applies an auth-aware limiter (`SSMA_OPTIMISTIC_REWORK_MAX` per `SSMA_OPTIMISTIC_REWORK_WINDOW_MS`) keyed by `userId:role`, logging `OPTIMISTIC_REWORK_RATE_LIMIT` when tripped.
- **Channel subscriptions**: `SyncGateway` tracks per-connection bursts (defaults: 8 subscribes per 10s, configurable via `SSMA_OPTIMISTIC_CHANNEL_*`). When exceeded, the client receives `{ type: 'channel.ack', status: 'error', code: 'RATE_LIMITED', retryAfterMs }` and the event is logged.
- All rate-limit hits, access denials, and server-generated rework/undo events flow through `LogService.ingestServerEvent`, so operators have centralized telemetry.

## Observability & admin tooling

- `LogService` buffers structured events (JSON lines or console) for: channel subscribe/unsubscribe, invalidations, server-side rework/undo, and rate-limit violations.
- **SSE** (`/optimistic/events`) still streams replay + invalidations for follower tabs, while DevTools consumes additional `channel.*` events for debugging.
- New admin APIs (staff or higher):
  - `GET /admin/optimistic/channels` — snapshot of every active subscription with connection id, params, role, site, and user metadata.
  - `GET /admin/optimistic/intents?reason=<filter>&limit=<n>` — enumerates pending intents with reason breakdowns, helping track stuck entries.

## Storage adapters

| Adapter | Activation | Notes |
|---------|------------|-------|
| File (default) | `SSMA_OPTIMISTIC_ADAPTER=file` | JSON file, simple to inspect, good for local dev.
| SQLite | `SSMA_OPTIMISTIC_ADAPTER=sqlite` + optional `SSMA_OPTIMISTIC_STORE` path | Uses `better-sqlite3` with WAL mode for single-node durability, preserves the same intent schema and reason semantics.

Adapter selection is seamless—both satisfy the `IntentStoreAdapter` contract (`append`, `entries`, `entriesSince`, `get`, `addReason`, `releaseReason`). Additional adapters (RocksDB, Postgres) can register via `registerIntentStoreAdapter(type, AdapterClass)`.

## Conflict resolution & CRDT metadata

- Every intent can now include `meta.reducer`, `meta.actionCreator`, `meta.actor`, and a structured `meta.crdt` descriptor (LWW register, G-Counter, or PN-Counter). The gateway validates and persists these fields verbatim so CSMA clients can deterministically reconcile server events.
- CSMA’s `ActionLogService` normalizes the descriptors before persistence, while the new `CrdtReducerRegistry` computes merged state from local writes, channel replays, and invalidations (e.g., inventory counters, cursor positions). This gives us Logux-style action creators + reducer ids with CRDT convergence semantics.
- `IntentStoreAdapter.normalizeMeta` guarantees every persisted entry retains the enriched metadata, so cross-tab leader replay or server resyncs provide the exact CRDT payloads required to rebuild deterministic state.

## Channel protocol parity

- `ChannelRegistry` specs now accept `filter`, `resend`, and custom `commands` handlers. Subscriptions remember per-client filter state, so snapshots/invalidations are scoped and resumable per connection and per `params`.
- `channel.command` messages (filter updates, resends, or bespoke commands) are round-tripped through `SyncGateway`, returning structured `{ type: 'channel.command', status }` responses plus server-initiated `channel.replay` payloads when resends succeed.
- Typed `channel.close` events (e.g., `ACCESS_DENIED`, `CLIENT_UNSUBSCRIBED`, `CONNECTION_CLOSED`) let CSMA differentiate between voluntary unsubscribes, auth changes, and server-initiated shutdowns, and DevTools displays the lifecycle events.
- Rate limiting applies to command bursts the same way it does for subscribe/unsubscribe, and all channel telemetry flows through `LogService` for observability.

## Configuration

Environment variables (see `src/config/env.js`):

| Var | Default | Description |
|-----|---------|-------------|
| `SSMA_OPTIMISTIC_STORE` | `data/optimistic-intents.json` | Storage path (JSON or SQLite DB file). |
| `SSMA_OPTIMISTIC_MAX_ENTRIES` | `5000` | Sliding cap before oldest intents are trimmed. |
| `SSMA_OPTIMISTIC_REPLAY_MS` | `300000` | Window used for replay snapshots + auto removal of `replay` reasons. |
| `SSMA_OPTIMISTIC_SUBPROTOCOL` | `1.0.0` | Semver negotiated with CSMA; bump major on breaking changes. |
| `SSMA_OPTIMISTIC_ADAPTER` | `file` | Selects persistence adapter (`file`, `sqlite`, custom). |
| `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` | `60000` | Rate-limit window for `/optimistic/rework` + `/optimistic/undo`. |
| `SSMA_OPTIMISTIC_REWORK_MAX` | `20` | Max requests per window per `(userId, role)` pair. |
| `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` | `10000` | Rate-limit window for `channel.subscribe` bursts. |
| `SSMA_OPTIMISTIC_CHANNEL_MAX` | `8` | Max subscribe attempts per window per connection/user. |

## Extending the Gateway

Most foundational pieces are complete (auth, RBAC, undo/rework, channel registry, SQLite adapter). Future enhancements to consider:

1. **Additional adapters**: Implement RocksDB, Postgres, or S3-backed adapters via `registerIntentStoreAdapter` for environments needing multi-node replication or cloud backups.
2. **Advanced auth flows**: Integrate password reset, email verification, or OAuth (e.g., [`oauth4webapi`](https://github.com/panva/oauth4webapi)) for enterprise deployments.
3. **Admin dashboard**: Build a UI (or CLI) powered by the `/admin/optimistic/*` APIs + log streams to inspect subscribers, intents, and rate-limit hits in real time.
4. **Observability sinks**: Forward `ingestServerEvent` batches into OpenTelemetry, ELK, or a metrics backend so operational KPIs (replay lag, intent throughput) are charted automatically.
5. **Channel policies**: Extend `ChannelRegistry` metadata with quotas, geographic filters, or multi-tenant enforcement aligned with CSMA channel usage.

## Testing

Vitest suites:

- `tests/intent-store.test.js` — covers reason tracking, persistence lifecycle, and replay expiration.
- `tests/optimistic-gateway.test.js` — spins up an in-memory gateway, verifies ACKs, persistence, subprotocol negotiation, and automatic replay on reconnect.

Use `npx vitest run` inside `SSMA/` to execute both.

## Operational Notes

- **Logs**: tie optimistic events into the existing `LogService` to record rate-limit hits, invalid batches, or subprotocol mismatches.
- **Scaling**: when moving to clustered deployments, put the WebSocket/SSE gateway behind a sticky load balancer or adopt a shared pub/sub (Redis, NATS) so invalidations propagate across nodes.
- **Security**: enforce `role=leader` for write operations, reject unknown roles, and throttle intent batches to prevent abuse.
