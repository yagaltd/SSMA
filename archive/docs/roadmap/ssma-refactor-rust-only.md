# SSMA Gateway: Rust-Only Implementation Tasks

**Date**: 2026-04-01
**Status**: Active
**Branch context**: `rust-gateway-consolidation`

## Summary

Consolidate on the Rust gateway (`apps/ssma-rust`) as the single SSMA runtime. The JS gateway (`apps/ssma-js`) stops receiving features and is archived once parity is confirmed. The gateway is a relay -- it fronts the business backend and third-party services (payments, email, cloud AI models), handling transport (HTTP/WS/SSE), auth, protocol validation, and fanout. It does not own business logic.

## Current state

Rust is ahead on the actively-developed surface (media, RTC, audio, WebRTC bridge). JS is ahead on operational scaffolding. Both share protocol contracts and conformance vectors.

| Surface | JS | Rust |
|---|---|---|
| WS/SSE transport | Yes | Yes |
| Intent store + replay | File + SQLite | File |
| Contract validation | AJV | jsonschema |
| JWT verify | Yes | Yes |
| JWT issuance + auth routes | Yes | No |
| RBAC | hasRole() utility | Inline checks |
| Rate limiting | Middleware | In-memory buckets |
| Backend adapter | fetch() | reqwest |
| Media assets | -- | Full CRUD + TTL |
| RTC signaling | -- | Yes |
| Audio sessions | -- | Yes |
| WebRTC bridge | -- | Experimental (may move to backend) |
| CORS | Yes | Yes |
| Optimistic rework/undo | Yes | Yes |
| Admin routes | Yes | Yes |
| Log relay | Sink to file | Relay + rate limit |
| Monitoring | HybridMonitor | Basic counters |

## File structure target

Current `gateway.rs` is 2,815 lines in a single file. Before adding features, split into handler modules:

```
src/
  main.rs              # entry point (exists)
  lib.rs               # module declarations (exists)
  config.rs            # Config struct + from_env (extract from runtime.rs)
  runtime.rs           # IntentStore, now_millis, now_secs (exists, trim)
  protocol.rs          # contract validation (exists)
  backend.rs           # BackendHttpClient (exists)
  gateway/
    mod.rs             # AppState, app() router builder, shared types
    ws.rs              # ws_session() and WS message dispatch
    sse.rs             # sse_events() handler
    auth.rs            # register, login, logout, me
    admin.rs           # channels listing, intents listing
    media.rs           # media asset CRUD + content serving
    rtc.rs             # RTC session + signal routes
    audio.rs           # audio session + command routes
    logs.rs            # /logs/batch relay handler
    optimistic.rs      # rework, undo, pending, metrics
    internal.rs        # /internal/backend/events, /internal/assets/*
  modules/
    mod.rs             # (exists)
    webrtc.rs          # experimental, may extract later
tests/
  conformance_runtime.rs   # (exists)
  e2e_scenarios.rs         # (exists, extend)
  store_and_backend.rs     # (exists)
  vector_harness.rs        # (exists)
  e2e_auth.rs              # new
  e2e_admin.rs             # new
  e2e_logs.rs              # new
  e2e_optimistic_ops.rs    # new
```

## Implementation tasks

### Phase 0: Modularize gateway.rs

Split the monolith into the file structure above. No behavior changes.

- [x] Extract `Config` and `from_env()` from `runtime.rs` into `config.rs`
- [x] Extract WS session logic into `gateway/ws.rs`
- [x] Extract SSE handler into `gateway/sse.rs`
- [x] Extract media asset handlers into `gateway/media.rs`
- [x] Extract RTC handlers into `gateway/rtc.rs`
- [x] Extract audio session handlers into `gateway/audio.rs`
- [x] Extract `/internal/*` routes into `gateway/internal.rs`
- [x] `gateway/mod.rs` holds `AppState`, `app()` router, shared types
- [x] All existing tests pass after refactor (no logic changes)

### Phase 1: Auth registration and login

Port JS `AuthService` + `UserStore` + `authRoutes` to Rust.

**Dependencies**: `argon2` crate (pure Rust, no native build)

- [x] Add `argon2` crate to `Cargo.toml`
- [x] Implement `UserStore` in `src/gateway/auth.rs` -- JSON file-backed, same shape as `data/users.json` (`{ users: [{ id, email, passwordHash, name, role, status, createdAt, updatedAt, lastLoginAt }] }`)
- [x] Add `SSMA_USER_STORE` env var to Config (default: `./data/users.json`)
- [x] Add `SSMA_JWT_ISSUER` env var to Config (default: `ssma-auth-service`)
- [x] Add `SSMA_JWT_AUDIENCE` env var to Config (default: `csma-clients`)
- [x] Add `SSMA_ACCESS_TTL_MS` env var to Config (default: `900000` = 15min)
- [x] Add `SSMA_AUTH_COOKIE_SECURE` env var to Config (default: `true`)
- [x] `POST /auth/register` -- argon2id hash, UserStore.create, issue JWT, set cookie
- [x] `POST /auth/login` -- verify hash, UserStore.update lastLoginAt, issue JWT, set cookie
- [x] `POST /auth/logout` -- clear cookie
- [x] `GET /auth/me` -- decode JWT from cookie, return user (strip passwordHash)
- [x] Auth errors: 400 (missing fields, password < 8 chars), 401 (bad credentials), 409 (email taken)
- [x] Cookie: httpOnly, sameSite=lax, path=/, secure=SSMA_AUTH_COOKIE_SECURE

**E2E test** (`tests/e2e_auth.rs`):

- [x] Register new user returns 201 + cookie + user (no passwordHash)
- [x] Duplicate email returns 409
- [x] Login with correct password returns 200 + cookie
- [x] Login with wrong password returns 401
- [x] GET /auth/me with valid cookie returns user
- [x] GET /auth/me without cookie returns 401
- [x] Logout clears cookie
- [x] Registered user JWT works for protected channel access

### Phase 2: CORS middleware

- [x] Add `tower-http` crate to `Cargo.toml`
- [x] Add `SSMA_ALLOWED_ORIGINS` env var to Config (default: `*`)
- [x] Add `tower_http::cors::CorsLayer` to `app()` router, configured from `SSMA_ALLOWED_ORIGINS` (comma-separated)
- [x] CORS applies to all routes including WS upgrade and SSE

**E2E test** (extend `e2e_scenarios.rs`):

- [x] Preflight OPTIONS returns correct `Access-Control-Allow-Origin`
- [x] Actual request includes CORS headers
- [x] Origin not in allowed list is rejected (when configured)

### Phase 3: Admin routes

Port JS `optimisticRoutes.js` admin endpoints.

- [x] `GET /admin/optimistic/channels` -- staff-only, returns channel subscription summary with connectionId, params, role, site
- [x] `GET /admin/optimistic/intents` -- staff-only, returns pending intents with reason filter support, limit cap at 500

**E2E test** (`tests/e2e_admin.rs`):

- [x] Staff JWT can list channels
- [x] Guest JWT gets 403 on channels
- [x] Staff JWT can list intents with default limit
- [x] `?limit=5` caps results
- [x] `?reason=rework` filters by reason

### Phase 4: Optimistic rework and undo

Port JS `optimisticRoutes.js` rework/undo endpoints.

- [x] `POST /optimistic/rework` -- staff-only, looks up intent by id, checks `meta.undo` exists, broadcasts rework via SSE/WS, returns 202
- [x] `POST /optimistic/undo` -- user-only, validates undo payload matches stored `meta.undo`, clears hold reasons, broadcasts undo, returns 200
- [x] `GET /optimistic/pending` -- returns intents from store (supports `?since=` cursor)

- [x] Add `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` env var to Config (default: `60000`)
- [x] Add `SSMA_OPTIMISTIC_REWORK_MAX` env var to Config (default: `20`)

**E2E test** (`tests/e2e_optimistic_ops.rs`):

- [x] Guest cannot rework (403)
- [x] Staff can rework an intent that has `meta.undo`
- [x] Rework intent without `meta.undo` returns 400
- [x] Rework unknown intent returns 404
- [x] User can undo own intent with matching payload
- [x] Undo with mismatched payload returns 409
- [x] Pending endpoint returns entries after cursor

### Phase 5: Telemetry relay

The gateway relays frontend telemetry to the business backend instead of sinking locally. CSMA's `LogAccumulator` sends `POST /logs/batch` with structured payloads every 30 seconds.

- [x] Add `SSMA_LOG_RELAY_URL` env var to Config -- backend endpoint for forwarded telemetry (default: `{backend_url}/logs/batch`)
- [x] Add `SSMA_BACKEND_TIMEOUT_MS` env var to Config (default: `5000`)
- [x] `POST /logs/batch` -- accept CSMA payload, forward to `SSMA_LOG_RELAY_URL` via BackendHttpClient, return 202
- [x] Rate-limit: 60 req/min per IP on `/logs/batch`
- [x] Gateway appends its own operational metrics (request counts, WS connections, rate limit hits) to each relayed batch in a `gateway` entry
- [x] `GET /logs/health` -- returns relay health (last relay timestamp, queue depth, error count)

**E2E test** (`tests/e2e_logs.rs`):

- [x] POST /logs/batch with valid payload returns 202
- [x] Toy backend receives the relayed batch
- [x] Batch includes gateway metrics entry
- [x] Rate limit kicks in after 60 requests in window
- [x] GET /logs/health returns relay status

### Phase 6: Transport tuning

Port JS transport-level configuration that the gateway needs as a relay for external services.

- [x] Add `SSMA_SSE_RETRY_MS` env var to Config (default: `2500`) -- SSE retry hint in KeepAlive
- [x] Add `SSMA_WS_MAX_BUFFERED_BYTES` env var to Config (default: `262144`) -- WS backpressure threshold, close slow consumers exceeding this
- [x] Add `SSMA_OPTIMISTIC_MAX_ENTRIES` env var to Config (default: `5000`) -- intent store cap before trimming oldest

**E2E test** (extend `e2e_scenarios.rs`):

- [x] SSE events include `retry:` field matching config
- [x] WS connection exceeding buffered bytes gets closed
- [x] Store trims to max entries when exceeded

### Phase 7: Graceful shutdown

- [x] Use `axum::serve` with `tokio::signal::ctrl_c()` graceful shutdown
- [x] Drain active WS connections before exiting (send close frames, wait up to 5s)
- [x] Flush pending SSE events
- [x] Persist intent store to disk before exit

**E2E test** (extend `e2e_scenarios.rs`):

- [x] SIGINT triggers graceful shutdown
- [x] Active WS connections receive close frame during shutdown
- [x] Intent store file is written on shutdown

### Phase 8: Archive JS gateway

After all phases complete and parity is confirmed.

- [x] Move `apps/ssma-js` to `archive/ssma-js`
- [x] Update `docs/roadmap/rust-parity-checklist.md` -- all items checked
- [x] Remove JS-specific scripts and workspace config from root `package.json`
- [x] Remove `templates/js-gateway/`
- [x] Update root `README.md` to reference Rust gateway only

## Environment variables

### Already in Rust (no action needed)

| Variable | Default | Usage |
|---|---|---|
| `SSMA_HOST` | `127.0.0.1` | Bind address |
| `SSMA_PORT` | `5050` | Bind port |
| `SSMA_PROTOCOL_SUBPROTOCOL` | `1.0.0` | WS subprotocol negotiation |
| `SSMA_OPTIMISTIC_SUBPROTOCOL` | `1.0.0` | Legacy alias |
| `SSMA_BACKEND_URL` | (empty) | Business backend URL |
| `SSMA_BACKEND_INTERNAL_TOKEN` | (empty) | Backend-to-gateway auth |
| `SSMA_AUTH_JWT_SECRET` | `change-me-in-production` | JWT signing key |
| `SSMA_AUTH_COOKIE` | `ssma_session` | Auth cookie name |
| `SSMA_ANON_COOKIE` | `ssma_anon` | Anonymous identity cookie |
| `SSMA_OPTIMISTIC_STORE` | `./data/optimistic-intents-rust.json` | Intent persistence path |
| `SSMA_OPTIMISTIC_REPLAY_MS` | `300000` | Replay window |
| `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` | `10000` | Channel subscribe rate window |
| `SSMA_OPTIMISTIC_CHANNEL_MAX` | `8` | Channel subscribe rate max |
| `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES` | `false` | Require JWT for intent writes |
| `SSMA_RATE_WINDOW_MS` | `60000` | Global rate limit window |
| `SSMA_RATE_MAX` | `120` | Global rate limit max |
| `SSMA_OPTIMISTIC_PROTECTED_CHANNELS` | (empty) | Comma-separated protected channel names |
| `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE` | `admin` | Min role for protected channels |
| `SSMA_MEDIA_STORAGE_ROOT` | `./data/media` | Media file storage directory |
| `SSMA_MEDIA_MAX_UPLOAD_BYTES` | `52428800` | Max upload size (50MB) |
| `SSMA_MEDIA_TTL_SECS` | `3600` | Media asset TTL |

### New variables (added across phases)

| Variable | Default | Phase | Usage |
|---|---|---|---|
| `SSMA_USER_STORE` | `./data/users.json` | 1 | User persistence path |
| `SSMA_JWT_ISSUER` | `ssma-auth-service` | 1 | JWT iss claim |
| `SSMA_JWT_AUDIENCE` | `csma-clients` | 1 | JWT aud claim |
| `SSMA_ACCESS_TTL_MS` | `900000` | 1 | Access token lifetime (15min) |
| `SSMA_AUTH_COOKIE_SECURE` | `true` | 1 | Set Secure flag on cookie |
| `SSMA_ALLOWED_ORIGINS` | `*` | 2 | CORS allowed origins (comma-separated) |
| `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` | `60000` | 4 | Rwork rate limit window |
| `SSMA_OPTIMISTIC_REWORK_MAX` | `20` | 4 | Rwork rate limit max |
| `SSMA_LOG_RELAY_URL` | `{backend_url}/logs/batch` | 5 | Backend telemetry endpoint |
| `SSMA_BACKEND_TIMEOUT_MS` | `5000` | 5 | Reqwest client timeout |
| `SSMA_SSE_RETRY_MS` | `2500` | 6 | SSE retry hint |
| `SSMA_WS_MAX_BUFFERED_BYTES` | `262144` | 6 | WS backpressure threshold |
| `SSMA_OPTIMISTIC_MAX_ENTRIES` | `5000` | 6 | Intent store size cap |

### JS variables not ported (and why)

| Variable | Reason |
|---|---|
| `SSMA_JWT_SECRET` (non-AUTH_) | JS had two secrets; Rust correctly uses `SSMA_AUTH_JWT_SECRET` only |
| `SSMA_HMAC_SECRET` / `HMAC_TTL_MS` | HMAC signing not used; JWT covers auth |
| `SSMA_REFRESH_TTL_MS` | Single token for now; add when refresh flow is needed |
| `SSMA_AUTH_JWT_EXPIRES_IN` | String format ("15m"); Rust uses `SSMA_ACCESS_TTL_MS` (millisecond int) |
| `SSMA_OPTIMISTIC_ADAPTER` | Rust has file adapter only; add when SQLite adapter is needed |
| `SSMA_LOG_EXPORTER` / `BUFFER_SIZE` / `FILE` / `MAX_BATCH` | Gateway relays, does not store locally |
| `SSMA_BACKEND_EMIT_EVENTS` | Rust always emits; add flag if needed |
| `SSMA_STATIC_RENDER_ENABLED` / `DISABLED_ISLANDS` | Frontend concern, not gateway's job |
| `SSMA_MONITOR_BACKLOG_THRESHOLD` / `INVALIDATION_BUDGET_MS` | Monitoring thresholds; add when monitoring matures beyond basic counters |
| `SSMA_SSE_MAX_QUEUE_BYTES` / `DRAIN_TIMEOUT_MS` | JS-specific backpressure; Rust SSE uses broadcast channel |
| `SSMA_WS_SLOW_CONSUMER_CLOSE_MS` | JS-specific; Rust WS handles via `SSMA_WS_MAX_BUFFERED_BYTES` |

## WebRTC experimental note

`modules/webrtc.rs` (907 lines) is experimental. The WebRTC bridge (rustrtc, voxudio, backend WS audio forwarding) may move to the business backend in a future iteration. For now, RTC signaling routes stay in the gateway; the bridge lives in the native binary only. No WASM considerations until this is resolved.

## CSMA is unaffected

CSMA (`/CSMA`) is a client-side vanilla JS framework. It communicates with SSMA over HTTP/WebSocket/SSE. The server runtime is invisible to it. The one integration point is `POST /logs/batch` -- Phase 5 ensures the Rust gateway handles the same payload shape CSMA already sends.
