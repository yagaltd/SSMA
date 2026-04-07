# Modules

## Shared Conceptual Modules

- transport gateway (WS/SSE + health/metrics)
- protocol validator (shared contracts)
- intent store (dedupe/replay/cursor)
- channel registry and fanout
- backend adapter client
- security policy (auth/RBAC/rate limits)

## JS Runtime Paths

- `apps/ssma-js/src/runtime/`
- `apps/ssma-js/src/services/optimistic/`
- `apps/ssma-js/src/backend/`
- `apps/ssma-js/src/routes/`

## Rust Runtime Paths

- `apps/ssma-rust/src/gateway.rs`
- `apps/ssma-rust/src/runtime.rs`
- `apps/ssma-rust/src/protocol.rs`
- `apps/ssma-rust/src/backend.rs`
