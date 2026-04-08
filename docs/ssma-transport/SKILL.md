---
name: ssma-transport
description: Work on HTTP endpoints, WebSocket handlers, SSE streams, or admin APIs. Use when modifying transport/ws.rs, transport/sse.rs, transport/mod.rs routes, or adding new endpoints.
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
| `/query/:name` | POST | `public_query()` | Forward query to backend adapter (JSON response) |
| `/query/:name/stream` | POST | `public_query_stream()` | Forward query with SSE streaming (NDJSON) |
| `/forms/submit` | POST | `features::forms::submit_form()` | Validate/rate-limit/honeypot/captcha/csrf hook then forward to backend (JSON, urlencoded, multipart fields) |
| `/webhooks/:provider` | POST | `features::webhooks::webhook_ingest()` | Verify provider payload + idempotency + backend forwarding |
| `/auth/register` | POST | `auth::register()` | Create account |
| `/auth/login` | POST | `auth::login()` | Authenticate |
| `/auth/refresh` | POST | `auth::refresh()` | Rotate refresh token and issue new session |
| `/auth/logout` | POST | `auth::logout()` | Clear session |
| `/auth/me` | GET | `auth::me()` | Current user profile |
| `/auth/forgot-password` | POST | `auth::forgot_password()` | Issue password-reset token via backend outbox hook |
| `/auth/reset-password` | POST | `auth::reset_password()` | Consume reset token and set new password |
| `/auth/verify-email` | POST | `auth::verify_email()` | Verify account email and activate login |
| `/auth/resend-verification` | POST | `auth::resend_verification()` | Re-issue verification token via backend outbox hook |
| `/auth/oidc/start` | GET | `auth::oidc_start()` | Begin OIDC auth code + PKCE flow |
| `/auth/oidc/callback` | GET | `auth::oidc_callback()` | Complete OIDC callback and issue session cookie |

### Media

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/media/assets` | POST | `media::upload_media()` | Upload binary (image/audio) |
| `/media/assets/:assetId` | GET | `media::get_asset_metadata()` | Asset metadata |
| `/media/assets/:assetId/content` | GET | `media::get_asset_content()` | Raw bytes |
| `/media/assets/:assetId` | DELETE | `media::delete_asset()` | Remove asset |

### RTC

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/rtc/sessions` | POST | `rtc::create_session()` | Create signaling session |
| `/rtc/sessions/:id/signals` | POST | `rtc::submit_signal()` | Offer/answer/candidate |

### Admin (staff+ required)

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/admin/optimistic/channels` | GET | `admin::admin_channels()` | Active subscriptions |
| `/admin/optimistic/intents` | GET | `admin::admin_intents()` | Pending intents |

### Internal (backend token required)

| Route | Method | Purpose |
|-------|--------|---------|
| `/internal/backend/events` | POST | Ingest backend events (invalidations and related backend-driven fanout) |
| `/internal/assets` | POST | Backend-created asset upload |
| `/internal/assets/:assetId` | GET | Asset metadata (backend access) |
| `/internal/assets/:assetId/content` | GET | Raw bytes (backend access) |
| `/internal/assets/:assetId` | DELETE | Remove asset (backend access) |

### Log Relay

| Route | Method | Handler | Purpose |
|-------|--------|---------|---------|
| `/logs/batch` | POST | `features::logs::logs_batch()` | Forward logs to relay URL |
| `/logs/health` | GET | `features::logs::logs_health()` | Relay status |

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

- `transport/mod.rs` — Router setup in `app()`, `build_state()`, helpers
- `transport/ws.rs` — WebSocket session loop, intent/channel handling
- `transport/sse.rs` — SSE stream construction
- `transport/admin.rs` — Staff-only endpoints
- `features/forms.rs` — Form handling ingress + anti-bot hooks
- `features/webhooks.rs` — Webhook verification + idempotency + forwarding
- `features/logs.rs` — Log relay forwarding
- `transport/internal.rs` — Backend event ingestion, internal assets

## Adding a New Route

1. Add handler function in the appropriate `transport/*.rs` or `features/*.rs` file
2. Register route in `transport/mod.rs` → `app()` function
3. Extract auth with `resolve_actor_from_headers()` or `resolve_user_from_headers()`
4. Emit server event with `emit_server_event()` for observability
5. Add E2E test in `tests/e2e_*.rs`
