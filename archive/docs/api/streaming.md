# Streaming Endpoints

Shared endpoints (JS + Rust):

- `GET /optimistic/ws` (WebSocket)
- `GET /optimistic/events` (SSE)

Write behavior:
- `intent.batch` accepted for `role=leader`
- optional auth requirement controlled by `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES`

Shared transport expectations:

- handshake emits `hello` then `replay`
- `hello` includes `connectionId` and the resolved `authRole`
- major-version subprotocol check is enforced
- invalid payloads return structured error frames
- channel subscribe flow emits `channel.ack` and `channel.snapshot`
- `channel.snapshot`, `channel.replay`, and `channel.invalidate` preserve the subscription `params`
- `channel.invalidate` fans out one `channel` at a time instead of a `channels[]` aggregate
- SSE island invalidations are filtered by RBAC and optional `?islands=` scoping
