# SSMA (Server-Side Microservices Architecture)

SSMA is a backend-agnostic realtime gateway implemented in both JavaScript and Rust.

It sits between frontend clients and your business backend, and owns:
- WebSocket and SSE transport
- replay and invalidation fanout
- auth and RBAC enforcement
- optimistic intent persistence
- backend adapter calls
- protocol validation and conformance

SSMA does not replace your application backend.
It provides the gateway contract and runtime behavior around it.

## Runtimes

- `apps/ssma-js`: Node.js runtime
- `apps/ssma-rust`: Rust runtime
- `packages/ssma-protocol`: shared contracts and vectors

## Core Contract

The canonical sources of truth are:
- [`docs/architecture/backend-interface.md`](docs/architecture/backend-interface.md)
- [`docs/protocol/wire-protocol.md`](docs/protocol/wire-protocol.md)
- [`docs/security/auth.md`](docs/security/auth.md)
- [`docs/security/rbac.md`](docs/security/rbac.md)

If code, tests, and docs diverge, align to the contract docs and update the rest.

## Quick Start

From the repo root:

```bash
npm install
npm run dev:js
```

For the Rust runtime:

```bash
cd apps/ssma-rust
cargo run
```

## Common Commands

```bash
npm run dev:js
npm run start:js
npm run test:js
npm run test:conformance
npm run run:e2e
npm run test:rust
npm run validate:templates
```

Targeted runs:

```bash
npm --prefix apps/ssma-js exec -- vitest run tests/<file>.test.js
cd apps/ssma-rust && cargo test --test <name> -- --nocapture
```

## Repository Map

| Path | Purpose |
| --- | --- |
| `apps/ssma-js/src/app.js` | JS runtime composition root |
| `apps/ssma-js/src/services/optimistic/SyncGateway.js` | JS WebSocket gateway |
| `apps/ssma-js/src/services/optimistic/OptimisticEventHub.js` | JS SSE fanout |
| `apps/ssma-js/src/backend/BackendHttpClient.js` | JS backend adapter client |
| `apps/ssma-rust/src/gateway.rs` | Rust gateway transport and fanout |
| `apps/ssma-rust/src/backend.rs` | Rust backend adapter client |
| `apps/ssma-rust/src/runtime.rs` | Rust config and intent store |
| `packages/ssma-protocol/contracts` | JSON contracts |
| `packages/ssma-protocol/vectors` | shared protocol vectors |
| `templates/` | CLI scaffold manifests |
| `docs/` | architecture, API, security, operations, testing |

## Documentation

Start here:
- [`docs/index.md`](docs/index.md)
- [`docs/README.md`](docs/README.md)

Recommended reading:
- [`docs/guides/SSMA-IN-A-NUTSHELL.md`](docs/guides/SSMA-IN-A-NUTSHELL.md)
- [`docs/guides/SSMA-RUNTIME.md`](docs/guides/SSMA-RUNTIME.md)
- [`docs/guides/SSMA-OPTIMISTIC-SYNC.md`](docs/guides/SSMA-OPTIMISTIC-SYNC.md)
- [`docs/api/streaming.md`](docs/api/streaming.md)
- [`docs/operations/config.md`](docs/operations/config.md)

## Templates

Available template manifests:
- `templates/js-gateway/template.manifest.json`
- `templates/rust-gateway/template.manifest.json`

Validate templates with:

```bash
npm run validate:templates
```

## Ecosystem Notes

Current ecosystem direction:
- backend starter support is good as an optional addon, not SSMA core
- Tauri support should start as an integration/template path
- a Tauri-specific transport/runtime should not be added until the architecture explicitly commits to it

See:
- [`roadmap.md`](roadmap.md)
- [`ssma_backend_starter.md`](ssma_backend_starter.md)
- [`ssma_tauri.md`](ssma_tauri.md)

## Agent Guidance

If this repo is used as a template for AI-assisted development, read [`AGENTS.md`](AGENTS.md) before making architectural or contract changes.
