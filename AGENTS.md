SSMA is a backend-agnostic realtime gateway. It owns:
- WebSocket and SSE transport
- replay and invalidation fanout
- auth and RBAC enforcement
- optimistic intent persistence
- backend adapter calls
- protocol validation and conformance

## Skills (Read Before Changing Code)

Each skill covers a specific domain. Read the relevant skill before making changes:

- `skills/ssma-overview/SKILL.md` — architecture, module layout, working conventions
- `skills/ssma-protocol/SKILL.md` — wire protocol, contracts, schema validation
- `skills/ssma-security/SKILL.md` — auth, RBAC, rate limits, CORS
- `skills/ssma-transport/SKILL.md` — HTTP endpoints, WS, SSE, admin APIs
- `skills/ssma-optimistic/SKILL.md` — intent store, replay, fanout, channels
- `skills/ssma-backend/SKILL.md` — backend adapter contract, media, events
- `skills/ssma-config/SKILL.md` — env vars, deployment, operations
- `skills/ssma-testing/SKILL.md` — how to write and run tests

## Runtime Map

- Rust runtime: `apps/ssma-rust`
- Shared contracts: `packages/ssma-protocol/contracts`
- Shared vectors: `packages/ssma-protocol/vectors`
- Template manifests: `templates/`

## Canonical Truths

- `ssma_session` is the session token cookie.
- auth identity comes from the verified session token, not a separate role cookie.
- guest access is valid for some flows.
- backend context is canonical camelCase JSON: `site`, `connectionId`, `ip`, `userAgent`, `user: { id, role } | null`.
- `channel.snapshot`, `channel.replay`, and `channel.invalidate` preserve subscription `params`.
- `channel.invalidate` targets one `channel`, not `channels[]`.
- shutdown must stop sockets, listeners, and reconnect loops cleanly.
- auth endpoints return `{ status: "ok", user: {...} }` envelope (matches CSMA `response.user`).
- `/query/:name` returns JSON, `/query/:name/stream` returns SSE with NDJSON chunks.

## Working Rules

1. Read the relevant skill file
2. Implement
3. Verify with `cargo test`
4. Update the skill file if behavior changes
5. Commit (never broken or undocumented)

## Commands

```bash
cd apps/ssma-rust && cargo test -- --nocapture
cd apps/ssma-rust && cargo test --test <name> -- --nocapture
npm run validate:templates
```

## Template Rules

- keep guidance generic enough for scaffolded projects
- prefer contract-first language over repo-local assumptions
- keep ecosystem addons optional

Do not turn SSMA core into:
- a provider-specific backend framework
- a Tauri-only transport layer
