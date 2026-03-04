# SSMA-Rust Parity Checklist

Track parity against JS gateway runtime.

- [x] Scaffold crate and module layout
- [x] Canonical subprotocol env support (`SSMA_PROTOCOL_SUBPROTOCOL`)
- [x] Shared contracts/vectors path wiring
- [x] WS `/optimistic/ws` endpoint
- [x] SSE `/optimistic/events` endpoint
- [x] Strict contract validation for all inbound message types
- [x] Intent store with replay cursor/logSeq
- [x] Backend adapter (`applyIntents`, `query`, `subscribe`, `health`)
- [x] E2E harness parity (scenarios A-F test suite added)
- [x] Golden-vector conformance pass (runtime vector replay tests added)
