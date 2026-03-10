SSMA is a backend-agnostic realtime gateway. It owns:
- WebSocket and SSE transport
- replay and invalidation fanout
- auth and RBAC enforcement
- optimistic intent persistence
- backend adapter calls
- protocol validation and conformance

## Runtime Map

- JS runtime: `apps/ssma-js`
- Rust runtime: `apps/ssma-rust`
- Shared contracts: `packages/ssma-protocol/contracts`
- Shared vectors: `packages/ssma-protocol/vectors`
- Template manifests: `templates/`

Important files:
- `apps/ssma-js/src/app.js`
- `apps/ssma-js/src/services/optimistic/SyncGateway.js`
- `apps/ssma-js/src/services/optimistic/OptimisticEventHub.js`
- `apps/ssma-js/src/backend/BackendHttpClient.js`
- `apps/ssma-rust/src/gateway.rs`
- `apps/ssma-rust/src/backend.rs`
- `apps/ssma-rust/src/runtime.rs`

## Read First

Before changing behavior, read:
- `docs/index.md`
- `docs/architecture/backend-interface.md`
- `docs/protocol/wire-protocol.md`
- `docs/security/auth.md`
- `docs/security/rbac.md`
- `docs/guides/SSMA-RUNTIME.md`

If touching channels or fanout, also read:
- `docs/guides/SSMA-OPTIMISTIC-SYNC.md`
- `docs/api/streaming.md`

## Canonical Truths

Assume these are intentional unless the task changes them:
- JS and Rust runtimes should stay behaviorally aligned.
- `ssma_session` is the session token cookie.
- auth identity comes from the verified session token, not a separate role cookie.
- guest access is valid for some flows.
- backend context is canonical camelCase JSON:
  - `site`
  - `connectionId`
  - `ip`
  - `userAgent`
  - `user: { id, role } | null`
- `channel.snapshot`, `channel.replay`, and `channel.invalidate` preserve subscription `params`.
- `channel.invalidate` targets one `channel`, not `channels[]`.
- SSE and WS island invalidations must both enforce RBAC.
- shutdown must stop sockets, listeners, and reconnect loops cleanly.

## Working Rules

Use this order for implementation tasks:
1. Implement
2. Verify
3. Update docs
4. Commit

Do not commit broken or undocumented behavior.
Breaking changes are acceptable in this development phase.
Do not add legacy fallback logic unless explicitly requested.

## Testing Rules

Prefer real unit and E2E coverage over mock-heavy tests.
When behavior changes, run the smallest relevant test set.

Useful commands:
- `npm run test:js`
- `npm run test:conformance`
- `npm run test:rust`
- `npm run validate:templates`
- `npm run check:docs`
- `npm --prefix apps/ssma-js exec -- vitest run tests/<file>.test.js`
- `cd apps/ssma-rust && cargo test --test <name> -- --nocapture`

If Rust tests live inside source files, keep them in `mod tests {}` at the bottom.

## Documentation Rules

When behavior changes, update the relevant docs in `docs/`.
At minimum consider:
- `docs/architecture/backend-interface.md`
- `docs/protocol/wire-protocol.md`
- `docs/security/auth.md`
- `docs/security/rbac.md`
- `docs/api/streaming.md`
- `docs/guides/SSMA-OPTIMISTIC-SYNC.md`
- `README.md`

## Template Rules

This repo is also a template source.
That means:
- keep guidance generic enough for scaffolded projects
- prefer contract-first language over repo-local assumptions
- keep ecosystem addons optional

Do not turn SSMA core into:
- a provider-specific backend framework
- a Tauri-only transport layer
- a one-runtime-only feature set

## Ecosystem Direction

Current direction:
- `ssma-backend-starter`: good as an optional addon, not core
- Tauri support: integration/template first
- no new CSMA Tauri transport module unless the architecture deliberately moves to Tauri IPC

See:
- `roadmap.md`
- `ssma_backend_starter.md`
- `ssma_tauri.md`

## Safety Checks

Before changing behavior:
- inspect code
- inspect docs
- inspect tests

Do not silently drift JS and Rust apart.
Do not add a new contract shape in only one runtime.
