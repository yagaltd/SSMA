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

The Rust runtime exposes:

- `POST /media/assets`
- `GET /media/assets/:assetId`
- `GET /media/assets/:assetId/content`
- `DELETE /media/assets/:assetId`
- `POST /rtc/sessions`
- `POST /rtc/sessions/:sessionId/signals`

Backend adapters can fetch SSMA-owned assets through:

- `GET /internal/assets/:assetId`
- `GET /internal/assets/:assetId/content`

Important behavior:

- frontend media is uploaded to SSMA, not directly to adapters
- adapters consume `assetId` references, not raw/base64 payloads
- guest-owned media and RTC sessions are bound to `SSMA_ANON_COOKIE` (`ssma_anon` by default)
- RTC signaling is ephemeral channel traffic and does not enter durable optimistic replay

## Templates

Available template manifests:
- `templates/rust-gateway/template.manifest.json`

Validate templates with:

```bash
npm run validate:templates
```

## Agent Guidance

If this repo is used as a template for AI-assisted development, read [`AGENTS.md`](AGENTS.md) before making architectural or contract changes.
