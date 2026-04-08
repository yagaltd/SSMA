---
name: ssma-overview
description: Understand SSMA architecture, module layout, and working conventions. Read this first before making any changes to the gateway.
---

# SSMA Gateway — Overview

SSMA is a backend-agnostic realtime gateway written in Rust. It sits between frontend clients and your business backend.

## What SSMA Owns

- WebSocket and SSE transport
- Auth and RBAC enforcement
- Intent persistence and replay
- Channel subscription fanout
- Protocol validation
- Media upload/download (images, audio)
- RTC signaling coordination
- Backend adapter forwarding

SSMA does **not** own business logic. It delegates to your backend through a narrow adapter contract.

## Architecture Layers

| Layer | Responsibility |
|-------|----------------|
| Protocol | Contracts and vectors from `packages/ssma-protocol` |
| Transport | HTTP health/metrics + WS/SSE sync endpoints |
| State | Intent store, cursor/log sequence, replay window, deduplication |
| Policy | Schema validation, auth extraction, RBAC, rate limits |
| Adapter | Backend HTTP client (`applyIntents`, `query`, `subscribe`, `health`) |
| Observability | Structured server events and transport metrics |

## Code Map

```
apps/ssma-rust/
├── src/
│   ├── main.rs              # Entry point
│   ├── lib.rs               # Module declarations
│   ├── config.rs            # Config::from_env() — all SSMA_* vars
│   ├── protocol.rs          # JSON schema validation against contracts
│   ├── domain/runtime.rs    # IntentStore — append, dedupe, replay, persist
│   ├── adapters/backend.rs  # BackendHttpClient — adapter calls
│   ├── transport/
│   │   ├── mod.rs           # AppState, Router, helpers, build_state()
│   │   ├── ws.rs            # WebSocket upgrade + session loop
│   │   ├── sse.rs           # SSE stream with replay
│   │   ├── auth.rs          # UserStore, register/login/logout/JWT
│   │   ├── admin.rs         # Staff-only channel/intent inspection
│   │   ├── logs.rs          # Log relay forwarding
│   │   └── internal.rs      # Backend-to-SSMA event ingestion
│   └── features/
│       ├── optimistic.rs    # Rework/undo/pending queries
│       ├── media.rs         # Asset upload/download/delete
│       ├── audio.rs         # Audio session management
│       ├── rtc.rs           # RTC signaling
│       └── webrtc.rs        # WebRTC bridge manager
├── tests/
│   ├── e2e_*.rs             # End-to-end tests
│   ├── conformance_runtime.rs
│   └── store_and_backend.rs
└── .env.example             # All config variables documented

packages/ssma-protocol/
├── contracts/               # JSON schemas (optimistic.json, channels.json, errors.json)
└── vectors/                 # Golden conformance vectors
```

## Working Rules

1. **Read first**: protocol docs, security docs, relevant SKILL.md
2. **Implement**: make the change
3. **Verify**: run `cargo test`
4. **Update docs**: if behavior changes, update the relevant doc
5. **Commit**: never commit broken or undocumented behavior

## Canonical Truths

- `ssma_session` is the session token cookie
- Auth identity comes from the verified JWT, not a separate role cookie
- Guest access is valid for some flows
- Backend context is canonical camelCase JSON: `site`, `connectionId`, `ip`, `userAgent`, `user: { id, role } | null`
- `channel.snapshot`, `channel.replay`, and `channel.invalidate` preserve subscription `params`
- `channel.invalidate` targets one `channel`, not `channels[]`
- Shutdown must stop sockets, listeners, and reconnect loops cleanly

## Commands

```bash
cd apps/ssma-rust && cargo run
cd apps/ssma-rust && cargo test -- --nocapture
cd apps/ssma-rust && cargo test --test <name> -- --nocapture
```
