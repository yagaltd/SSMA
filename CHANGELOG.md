# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-04-08

### Added

- Rust gateway: auth, CORS, media upload/download, RTC signaling, telemetry relay with rate limiting
- `/ready` readiness endpoint
- Structured transport/server event logging
- E2E test suite (auth, CORS, health, log relay, WS transport, query streaming)

### Changed

- Reorganized Rust runtime into domain/adapters/transport/features layers
- Split gateway monolith into focused modules
- Hardened single-node behavior: startup validation, timeout policy, graceful shutdown, WS backpressure
- Clarified project scope as a backend-agnostic single-node gateway template
- Reworked docs into focused `docs/ssma-*/SKILL.md` guidance files

### Fixed

- Auth endpoints now return `{ status: "ok", user: ... }` for CSMA compatibility
- NDJSON streaming: reassembles split lines, forwards multiple objects per chunk
- Atomic file writes now call `sync_all()`
- Security defaults, docs consistency, tests, and repo hygiene audit fixes

### Removed

- Legacy JS gateway (`apps/ssma-js/`)
- Stale templates, scripts, archived docs, and non-active audio/WebRTC surface
- Tracked `roadmap.md` (now local-only)

## [0.1.0] - 2024-03

### Added

- Initial single-node gateway template
- WebSocket and SSE transport
- Basic auth and RBAC enforcement
- Channel subscription fanout
- Backend adapter forwarding
- Protocol validation and conformance
