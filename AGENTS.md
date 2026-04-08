# AGENTS.md

Read before changing code.

## What SSMA Is

Rust gateway between frontend clients and business backend. Owns transport, auth, persistence, fanout. Does not own business logic.

## Docs

Each doc covers a domain. Read relevant doc before changes:

- `docs/ssma-overview/SKILL.md` — architecture, code map, conventions
- `docs/ssma-protocol/SKILL.md` — wire protocol, contracts, validation
- `docs/ssma-security/SKILL.md` — auth, RBAC, rate limits, CORS
- `docs/ssma-transport/SKILL.md` — HTTP, WS, SSE endpoints
- `docs/ssma-optimistic/SKILL.md` — intent store, replay, fanout
- `docs/ssma-backend/SKILL.md` — backend adapter contract
- `docs/ssma-config/SKILL.md` — env vars, deployment
- `docs/ssma-testing/SKILL.md` — how to test

## Code Map

```
apps/ssma-rust/src/
├── main.rs              Entry point
├── config.rs            Config::from_env()
├── protocol.rs          JSON schema validation
├── runtime.rs           IntentStore
├── backend.rs           BackendHttpClient
├── domain/runtime.rs    IntentStore
├── adapters/backend.rs  BackendHttpClient
├── transport/           AppState, router, WS/SSE/auth/admin/internal
└── features/            optimistic, media, rtc, logs
```

## Canonical Truths

- `ssma_session` = session cookie name
- Auth identity from JWT, not separate role cookie
- Guest access valid for some flows
- Backend context: `{ site, connectionId, ip, userAgent, user: { id, role } | null }`
- `channel.snapshot`, `channel.replay`, `channel.invalidate` preserve `params`
- `channel.invalidate` targets one `channel`, not `channels[]`
- Auth endpoints return `{ status: "ok", user: {...} }` envelope
- `/query/:name` = JSON, `/query/:name/stream` = SSE NDJSON
- Shutdown must stop sockets, listeners, reconnect loops

## Working Rules

1. Read relevant doc
2. Implement
3. `cargo test`
4. Update doc if behavior changes
5. Commit (never broken or undocumented)

## Commands

```bash
cd apps/ssma-rust && cargo test -- --nocapture
cd apps/ssma-rust && cargo test --test <name> -- --nocapture
```
