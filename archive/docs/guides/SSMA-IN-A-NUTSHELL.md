# SSMA in a Nutshell

SSMA (Server-Side Microservices Architecture) is a backend gateway that sits between clients and domain backends.
This repository contains two runtime implementations:

- JS: `apps/ssma-js`
- Rust: `apps/ssma-rust`

Both runtimes share protocol definitions from `packages/ssma-protocol`.

## Objectives

1. **Protocol parity**: both runtimes must emit compatible WS/SSE frames.
2. **Gateway authority**: validate, authorize, persist, forward, and fan out invalidations.
3. **Backend-agnostic design**: domain logic is delegated through adapter interfaces.
4. **Security-first defaults**: auth, RBAC, and rate-limit controls on transport paths.

## Core Building Blocks

- **Protocol layer**: canonical JSON contracts and vectors in `packages/ssma-protocol`.
- **Transport layer**: WS (`/optimistic/ws`) + SSE (`/optimistic/events`) + health/metrics routes.
- **State layer**: intent persistence, replay cursor/log sequence, idempotency and dedupe.
- **Policy layer**: auth extraction, role checks, rate limiting, protected-channel controls.
- **Backend adapter layer**: `applyIntents`, `query`, `subscribe`, `health`, and backend event ingestion.

## Intent Flow (Cross-Runtime)

1. Client sends `intent.batch` to `/optimistic/ws`.
2. Gateway validates payload against shared contract.
3. Gateway enforces auth/RBAC/rate limits.
4. Gateway persists intents and dedupes by `(site, id)`.
5. Gateway forwards only fresh intents to backend adapter.
6. Gateway sends ACK status and emits invalidation events to WS/SSE subscribers.

## Runtime Notes

- JS runtime also includes auth-centric HTTP routes (`/auth/*`, `/logs/*`) and a Node middleware kernel.
- Rust runtime currently focuses on optimistic gateway parity and transport semantics.

## Shared Protocol Source of Truth

- Contracts: `packages/ssma-protocol/contracts`
- Vectors: `packages/ssma-protocol/vectors`

## Quick Start (Monorepo)

```bash
npm run test:conformance
npm run test:rust
```
