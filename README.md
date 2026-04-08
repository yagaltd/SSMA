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
- Generic form ingress handling (JSON/urlencoded/multipart + honeypot/captcha/csrf hooks + backend forward)
- Generic webhook ingress handling (verification hook + idempotency + backend forward)
- OIDC client bridge endpoints
- Auth lifecycle endpoints (refresh, forgot/reset password, email verification)
- Backend adapter forwarding
- Protocol validation

## Quick Start

```bash
cd apps/ssma-rust
cargo run
```

Requires `SSMA_AUTH_JWT_SECRET` in production. See `apps/ssma-rust/.env.example`.

## Single-Node Production

SSMA core now targets a clean single-node gateway deployment:
- file-backed optimistic intent persistence
- file-backed user store
- `/ready` for operator readiness checks
- bounded backend request timeouts
- graceful shutdown for WS/SSE clients
- WS backpressure protection

Recommended deployment model:
1. run behind HTTPS reverse proxy
2. keep `SSMA_AUTH_COOKIE_SECURE=true`
3. set strong `SSMA_AUTH_JWT_SECRET`
4. mount persistent `./data/` storage
5. treat SSMA as gateway only; business persistence stays in your backend

Out of scope for SSMA core:
- opinionated SQLite/Postgres business backend
- admin product UI
- durable business workflows

Those belong in future `examples/`.

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
│   ├── domain/runtime.rs IntentStore (persist, dedupe, replay)
│   ├── adapters/backend.rs BackendHttpClient (adapter calls)
│   ├── transport/       Inbound HTTP/WS/SSE transport
│   └── features/        Feature handlers (forms, media, rtc, logs, optimistic)
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

## Docs

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
- `GET /ready`
- `GET /optimistic/metrics`
- `GET /optimistic/ws` (WebSocket)
- `GET /optimistic/events` (SSE)

### Auth
- `POST /auth/register`
- `POST /auth/login`
- `POST /auth/refresh`
- `POST /auth/logout`
- `GET /auth/me`
- `POST /auth/forgot-password`
- `POST /auth/reset-password`
- `POST /auth/verify-email`
- `POST /auth/resend-verification`
- `GET /auth/oidc/start`
- `GET /auth/oidc/callback`

### Query
- `POST /query/:name` (JSON)
- `POST /query/:name/stream` (SSE)

### Forms
- `POST /forms/submit` (`application/json`, `application/x-www-form-urlencoded`, `multipart/form-data`)

### Webhooks
- `POST /webhooks/:provider`

### Media
- `POST /media/assets`
- `GET /media/assets/:assetId`
- `GET /media/assets/:assetId/content`
- `DELETE /media/assets/:assetId`

### RTC
- `POST /rtc/sessions`
- `POST /rtc/sessions/:sessionId/signals`

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
