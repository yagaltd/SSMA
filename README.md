# SSMA (Server-Side Microservices Architecture)

SSMA is a backend-agnostic realtime gateway implemented in Rust.

It sits between frontend clients and your business backend, and owns:
- WebSocket and SSE transport
- media ingress and asset ownership
- replay and invalidation fanout
- auth and RBAC enforcement
- optimistic intent persistence
- RTC signaling/session coordination
- backend adapter calls
- protocol validation and conformance

SSMA does not replace your application backend.
It provides the gateway contract and runtime behavior around it.

## Runtime

- `apps/ssma-rust`: Rust runtime
- `packages/ssma-protocol`: shared contracts and vectors

The JS gateway (`apps/ssma-js`) has been archived to `archive/ssma-js`.

## Core Contract

The canonical sources of truth are the **skills**:

- [`skills/ssma-protocol/SKILL.md`](skills/ssma-protocol/SKILL.md) — wire protocol, message contracts
- [`skills/ssma-security/SKILL.md`](skills/ssma-security/SKILL.md) — auth, RBAC, rate limits
- [`skills/ssma-backend/SKILL.md`](skills/ssma-backend/SKILL.md) — backend adapter contract
- [`skills/ssma-transport/SKILL.md`](skills/ssma-transport/SKILL.md) — HTTP, WS, SSE endpoints

If code, tests, and skills diverge, align to the skill docs and update the rest.

## Quick Start

```bash
cd apps/ssma-rust
cargo run
```

## Common Commands

```bash
cd apps/ssma-rust && cargo test -- --nocapture
npm run validate:templates
```

Targeted test runs:

```bash
cd apps/ssma-rust && cargo test --test <name> -- --nocapture
```

## Skills

Read before changing code:

| Skill | Covers |
|-------|--------|
| [`ssma-overview`](skills/ssma-overview/SKILL.md) | Architecture, module layout, conventions |
| [`ssma-protocol`](skills/ssma-protocol/SKILL.md) | Wire protocol, contracts, schema validation |
| [`ssma-security`](skills/ssma-security/SKILL.md) | Auth, RBAC, rate limits, CORS |
| [`ssma-transport`](skills/ssma-transport/SKILL.md) | HTTP endpoints, WS, SSE, admin APIs |
| [`ssma-optimistic`](skills/ssma-optimistic/SKILL.md) | Intent store, replay, fanout, channels |
| [`ssma-backend`](skills/ssma-backend/SKILL.md) | Backend adapter contract, media, events |
| [`ssma-config`](skills/ssma-config/SKILL.md) | Env vars, deployment, operations |
| [`ssma-testing`](skills/ssma-testing/SKILL.md) | How to write and run tests |

## Repository Map

| Path | Purpose |
| --- | --- |
| `apps/ssma-rust/src/gateway/` | Rust gateway transport and fanout |
| `apps/ssma-rust/src/backend.rs` | Rust backend adapter client |
| `apps/ssma-rust/src/config.rs` | Rust configuration |
| `apps/ssma-rust/src/runtime.rs` | Rust intent store |
| `packages/ssma-protocol/contracts` | JSON contracts |
| `packages/ssma-protocol/vectors` | shared protocol vectors |
| `templates/` | CLI scaffold manifests |
| `skills/` | AI-agent skills (read before changing code) |

## Gateway Surface

### Transport

- `GET /health` — service health + cursor
- `GET /optimistic/metrics` — operational counters
- `GET /optimistic/ws` — WebSocket sync (leader/follower)
- `GET /optimistic/events` — SSE stream

### Auth

- `POST /auth/register` → `{ status, user }`
- `POST /auth/login` → `{ status, user }`
- `POST /auth/logout` → `{ status }`
- `GET /auth/me` → `{ status, user }`

### Query

- `POST /query/:name` — JSON query to backend adapter
- `POST /query/:name/stream` — SSE streaming (NDJSON from backend)

### Media

- `POST /media/assets` — upload binary (image/audio)
- `GET /media/assets/:assetId` — metadata
- `GET /media/assets/:assetId/content` — raw bytes
- `DELETE /media/assets/:assetId` — remove

### RTC & Audio

- `POST /rtc/sessions` — create signaling session
- `POST /rtc/sessions/:sessionId/signals` — submit signal
- `POST /audio/sessions` — create audio session
- `GET /audio/sessions/:sessionId` — session metadata
- `DELETE /audio/sessions/:sessionId` — end session
- `POST /audio/sessions/:sessionId/commands` — start/pause/resume/stop

### Admin (staff+)

- `GET /admin/optimistic/channels` — active subscriptions
- `GET /admin/optimistic/intents` — pending intents

### Logs

- `POST /logs/batch` — forward logs to relay URL
- `GET /logs/health` — relay status

### Internal (backend token required)

- `POST /internal/backend/events` — ingest backend events
- `POST /internal/assets` — backend-created asset upload
- `GET /internal/assets/:assetId` — metadata
- `GET /internal/assets/:assetId/content` — raw bytes
- `DELETE /internal/assets/:assetId` — remove

Important behavior:

- frontend media is uploaded to SSMA, not directly to adapters
- adapters consume `assetId` references, not raw/base64 payloads
- guest-owned media and RTC sessions are bound to `SSMA_ANON_COOKIE` (`ssma_anon` by default)
- RTC signaling is ephemeral channel traffic and does not enter durable optimistic replay
- auth endpoints return `{ status: "ok", user: {...} }` envelope for CSMA compatibility
- streaming queries use SSE with NDJSON chunks from backend

## Templates

Available template manifests:
- `templates/rust-gateway/template.manifest.json`

Validate templates with:

```bash
npm run validate:templates
```

## Agent Guidance

If this repo is used as a template for AI-assisted development, read [`AGENTS.md`](AGENTS.md) before making architectural or contract changes.
