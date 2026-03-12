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
- [x] Public query route (`POST /query/:name`)
- [x] E2E harness parity (scenarios A-F test suite added)
- [x] Golden-vector conformance pass (runtime vector replay tests added)
- [x] SSMA-owned media asset routes (`/media/*`)
- [x] Backend-token-protected internal asset fetch (`/internal/assets/*`)
- [x] Backend-token-protected internal asset creation/deletion (`POST/DELETE /internal/assets/*`)
- [x] Anonymous guest ownership for media assets (`SSMA_ANON_COOKIE`)
- [x] RTC signaling routes (`/rtc/sessions`, `/rtc/sessions/:id/signals`)
- [x] Durable vs ephemeral split validated in Rust E2E (RTC stays out of replay)

Pending JS parity:
- [ ] `/media/*` routes
- [ ] `/internal/assets/*` adapter fetch routes
- [ ] RTC signaling routes
- [ ] Anonymous guest asset ownership parity
