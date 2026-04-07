---
name: ssma-transport
description: Work on HTTP endpoints, WebSocket handlers, SSE streams, or admin APIs. Use when modifying gateway/ws.rs, gateway/sse.rs, gateway/mod.rs routes, or adding new endpoints.
---

# SSMA Transport

## HTTP Endpoints

### Public

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/health` | GET | `health()` | Service health + subprotocol + cursor |
| `/optimistic/metrics` | GET | `metrics()` | Operational counters |
| `/optimistic/ws` | GET | `ws::ws_upgrade()` | WebSocket upgrade |
| `/optimistic/events` | GET | `sse::sse_events()` | SSE stream |
| `/query/:name` | POST | `public_query()` | Forward query to backend adapter |
| `/auth/register` | POST | `auth::register()` | Create account |
| `/auth/login` | POST | `auth::login()` | Authenticate |
| `/auth/logout` | POST | `auth::logout()` | Clear session |
| `/auth/me` | GET | `auth::me()` | Current user profile |

### Media

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/media/assets` | POST | `media::upload_media()` | Upload binary (image/audio) |
| `/media/assets/:assetId` | GET | `media::get_asset_metadata()` | Asset metadata |
| `/media/assets/:assetId/content` | GET | `media::get_asset_content()` | Raw bytes |
| `/media/assets/:assetId` | DELETE | `media::delete_asset()` | Remove asset |

### RTC & Audio

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/rtc/sessions` | POST | `rtc::create_session()` | Create signaling session |
| `/rtc/sessions/:id/signals` | POST | `rtc::submit_signal()` | Offer/answer/candidate |
| `/audio/sessions` | POST | `audio::create_session()` | Create audio session |
| `/audio/sessions/:id` | GET | `audio::get_session()` | Session metadata |
| `/audio/sessions/:id` | DELETE | `audio::delete_session()` | End session |
| `/audio/sessions/:id/commands` | POST | `audio::command_session()` | start/pause/resume/stop |

### Admin (staff+ required)

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/admin/optimistic/channels` | GET | `admin::admin_channels()` | Active subscriptions |
| `/admin/optimistic/intents` | GET | `admin::admin_intents()` | Pending intents |

### Internal (backend token required)

| Route | Method | Purpose |
|-------|--------|---------|
| `/internal/backend/events` | POST | Ingest backend events (invalidations, audio) |
| `/internal/assets` | POST | Backend-created asset upload |
| `/internal/assets/:assetId` | GET | Asset metadata (backend access) |
| `/internal/assets/:assetId/content` | GET | Raw bytes (backend access) |
| `/internal/assets/:assetId` | DELETE | Remove asset (backend access) |

### Log Relay

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/logs/batch` | POST | `logs::logs_batch()` | Forward logs to relay URL |
| `/logs/health` | GET | `logs::logs_health()` | Relay status |

## WebSocket Session

### Connection

```
GET /optimistic/ws?role=leader&site=<site>&subprotocol=1.0.0
```

Query params:
- `role` — `leader` (can write) or `follower` (default, read-only)
- `site` — site identifier (default `default`)
- `subprotocol` — client subprotocol version
- `cursor` — replay cursor for resumption

### Session Loop

1. Server sends `hello` (connectionId, subprotocol, authRole, serverTime)
2. Server sends `replay` (recent intents + cursor)
3. Client sends messages (`intent.batch`, `channel.subscribe`, etc.)
4. Server processes and responds (`ack`, `channel.ack`, `channel.snapshot`)
5. Server broadcasts invalidations as they occur

### Backpressure

When outbound buffer exceeds `SSMA_WS_MAX_BUFFERED_BYTES` (default 256 KB):
- Server sends `error` with code `BACKPRESSURE`
- Server closes the connection

## SSE Stream

```
GET /optimistic/events?site=<site>&cursor=<n>&islands=a,b
```

- Sends `ready` → `replay` → ongoing `invalidate` / `island.invalidate` events
- Keep-alive interval: `SSMA_SSE_RETRY_MS` (default 2.5s)
- RBAC filters island invalidations by role + `?islands=` scoping
- SSE never delivers ACKs (read-only transport)

## Metrics Response

```json
{
  "status": "ok",
  "service": "ssma-rust",
  "active": { "ws": 5, "sse": 12 },
  "totals": {
    "wsConnections": 1000,
    "sseConnections": 500,
    "broadcasts": 50000,
    "rateLimitHits": 42,
    "sseClientDropped": 3,
    "wsUnauthorizedFiltered": 1,
    "sseUnauthorizedFiltered": 0
  },
  "store": { "cursor": 12345, "replayDepth": 150 },
  "serverEvents": { "INTENT_ACKED": 300, "CHANNEL_SUBSCRIBE": 200 }
}
```

## Key Files

- `gateway/mod.rs` — Router setup in `app()`, `build_state()`, helpers
- `gateway/ws.rs` — WebSocket session loop, intent/channel handling
- `gateway/sse.rs` — SSE stream construction
- `gateway/admin.rs` — Staff-only endpoints
- `gateway/logs.rs` — Log relay forwarding
- `gateway/internal.rs` — Backend event ingestion, internal assets

## Adding a New Route

1. Add handler function in the appropriate `gateway/*.rs` file
2. Register route in `mod.rs` → `app()` function
3. Extract auth with `resolve_actor_from_headers()` or `resolve_user_from_headers()`
4. Emit server event with `emit_server_event()` for observability
5. Add E2E test in `tests/e2e_*.rs`
