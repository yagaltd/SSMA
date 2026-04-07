# E2E Harness

Reference tests:

- JS: `apps/ssma-js/tests/gateway-backend-e2e.test.js`
- Rust: `apps/ssma-rust/tests/e2e_scenarios.rs`

Components:
- real SSMA server
- real toy backend (`apps/ssma-js/examples/toy-backend/server.mjs`)
- real WS client + SSE stream parser

Core assertions:
- handshake returns `hello` with negotiated subprotocol
- replay frame is emitted on connect (empty or populated)
- replay cursor/log sequence remains monotonic across reconnects
- write -> backend apply -> ACK + invalidate
- idempotent retry does not double-apply
- unauthorized writes are rejected when auth required
