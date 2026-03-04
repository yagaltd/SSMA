# SSMA-Rust

This directory contains the Rust optimistic gateway implementation for SSMA protocol parity.

## Current state

- Runnable Axum gateway runtime
- WS endpoint `/optimistic/ws` and SSE endpoint `/optimistic/events`
- Metrics endpoint `/optimistic/metrics`
- Internal backend event ingest endpoint `/internal/backend/events`
- Strict inbound message validation via shared schemas in `../../packages/ssma-protocol/contracts`
- File-backed intent store with `(site,id)` dedupe and monotonic `log_seq`
- Backend HTTP client methods (`apply_intents`, `query`, `subscribe`, `health`)
- Channel subscription registry and WS `channel.invalidate` fanout
- Protected-channel RBAC checks + global/channel rate limiting
- Structured server-event counters in metrics
- Rust conformance runtime tests replaying shared vectors
- Rust E2E scenario suite covering handshake, write/ack/invalidate, idempotency, auth reject, channel snapshot, and subprotocol mismatch

## Planned parity milestones

1. Implement WS endpoint `/optimistic/ws`
2. Implement SSE endpoint `/optimistic/events`
3. Add strict JSON schema validation for inbound frames
4. Add intent store (sqlite or file)
5. Add backend HTTP client (`applyIntents`, `query`, `subscribe`, `health`)
6. Run shared vector conformance suite

## Environment

- `SSMA_PROTOCOL_SUBPROTOCOL` is the canonical protocol setting.
- `SSMA_BACKEND_INTERNAL_TOKEN` protects `/internal/backend/events` when set.
- `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES=true` enforces authenticated leader writes.
- `SSMA_RATE_WINDOW_MS` and `SSMA_RATE_MAX` configure global WS message rate limits.
- `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` and `SSMA_OPTIMISTIC_CHANNEL_MAX` configure subscribe burst limits.
- `SSMA_OPTIMISTIC_PROTECTED_CHANNELS` configures comma-separated protected channels.
- `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE` configures minimum role for protected channels.
- Rust reads process environment directly (no built-in dotenv loader).
- A starter template is provided at `apps/ssma-rust/.env.example`.

## Quickstart (Local)

1. Set minimal env vars:
   - `SSMA_PORT=5050`
   - `SSMA_PROTOCOL_SUBPROTOCOL=1.0.0`
   - `SSMA_BACKEND_URL=http://127.0.0.1:6060` (or leave empty for local fallback behavior)
2. Optional backend token wiring:
   - `SSMA_BACKEND_INTERNAL_TOKEN=<token>`
3. Optional shell loading from `.env`:
   - `set -a && source .env && set +a`
4. Run Rust gateway:
   - `cd apps/ssma-rust`
   - `cargo run`
5. Check health:
   - `curl http://127.0.0.1:5050/health`
6. Check metrics:
   - `curl http://127.0.0.1:5050/optimistic/metrics`

## Local Validation Commands

From `apps/ssma-rust/`:

- Unit + conformance + E2E tests:
  - `cargo test -- --nocapture`

From repository root:

- JS conformance and docs checks used by CI:
  - `npm --prefix apps/ssma-js run check:docs`
  - `npm --prefix apps/ssma-js run test:conformance`

## Notes

- In restricted/offline environments, `cargo` may fail to download crates from `crates.io`.
- WS endpoint: `/optimistic/ws`
- SSE endpoint: `/optimistic/events`
- Backend event ingest endpoint: `/internal/backend/events`
