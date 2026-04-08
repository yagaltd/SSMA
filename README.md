# SSMA

Backend-agnostic realtime gateway in Rust.

Sits between frontend clients and your business backend. Owns transport, auth, persistence, fanout. Does not own business logic.

## What SSMA Does

- WebSocket + SSE transport
- Auth + RBAC enforcement
- Optimistic intent persistence + replay
- Channel subscription fanout
- Media upload/download
- RTC signaling coordination
- Backend adapter forwarding
- Protocol validation

## Quick Start

```bash
cd apps/ssma-rust
cargo run
```

Requires `SSMA_AUTH_JWT_SECRET` in production. See `apps/ssma-rust/.env.example`.

## Commands

```bash
cargo test --manifest-path apps/ssma-rust/Cargo.toml -- --nocapture
cargo test --manifest-path apps/ssma-rust/Cargo.toml --test <name> -- --nocapture
```

## Project Structure

```
apps/ssma-rust/          Gateway binary
├── src/
│   ├── main.rs          Entry point
│   ├── config.rs        Config::from_env()
│   ├── protocol.rs      JSON schema validation
│   ├── runtime.rs       IntentStore (persist, dedupe, replay)
│   ├── backend.rs       BackendHttpClient (adapter calls)
│   └── gateway/
│       ├── mod.rs       AppState, router, helpers
│       ├── ws.rs        WebSocket session
│       ├── sse.rs       SSE stream
│       ├── auth.rs      Register/login/JWT
│       ├── media.rs     Asset upload/download
│       ├── admin.rs     Staff endpoints
│       ├── optimistic.rs Rework/undo
│       ├── logs.rs      Log relay
│       └── internal.rs  Backend event ingestion
└── tests/               E2E + unit tests

packages/ssma-protocol/  Shared contracts + vectors
├── contracts/           JSON schemas
└── vectors/             Golden conformance vectors

docs/                  AI-agent instructions
├── ssma-overview/       Architecture, code map
├── ssma-protocol/       Wire protocol, contracts
├── ssma-security/       Auth, RBAC, rate limits
├── ssma-transport/      HTTP, WS, SSE endpoints
├── ssma-optimistic/     Intent store, replay, fanout
├── ssma-backend/        Backend adapter contract
├── ssma-config/         Env vars, deployment
└── ssma-testing/        How to write tests
```

## Skills

Read before changing code:

| Skill | Covers |
|-------|--------|
| `ssma-overview` | Architecture, module layout, conventions |
| `ssma-protocol` | Wire protocol, contracts, schema validation |
| `ssma-security` | Auth, RBAC, rate limits, CORS |
| `ssma-transport` | HTTP endpoints, WS, SSE, admin APIs |
| `ssma-optimistic` | Intent store, replay, fanout, channels |
| `ssma-backend` | Backend adapter contract, media, events |
| `ssma-config` | Env vars, deployment, operations |
| `ssma-testing` | How to write and run tests |

## Gateway Endpoints

### Transport
- `GET /health`
- `GET /optimistic/metrics`
- `GET /optimistic/ws` (WebSocket)
- `GET /optimistic/events` (SSE)

### Auth
- `POST /auth/register`
- `POST /auth/login`
- `POST /auth/logout`
- `GET /auth/me`

### Query
- `POST /query/:name` (JSON)
- `POST /query/:name/stream` (SSE)

### Media
- `POST /media/assets`
- `GET /media/assets/:assetId`
- `GET /media/assets/:assetId/content`
- `DELETE /media/assets/:assetId`

### RTC & Audio
- `POST /rtc/sessions`
- `POST /rtc/sessions/:sessionId/signals`
- `POST /audio/sessions`
- `GET /audio/sessions/:sessionId`
- `DELETE /audio/sessions/:sessionId`
- `POST /audio/sessions/:sessionId/commands`

### Admin (staff+)
- `GET /admin/optimistic/channels`
- `GET /admin/optimistic/intents`

### Internal (backend token)
- `POST /internal/backend/events`
- `POST /internal/assets`
- `GET /internal/assets/:assetId`
- `GET /internal/assets/:assetId/content`
- `DELETE /internal/assets/:assetId`

## License

MIT
