# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-04-08

### Added

- Auth lifecycle endpoints:
  - `POST /auth/refresh`
  - `POST /auth/forgot-password`
  - `POST /auth/reset-password`
  - `POST /auth/verify-email`
  - `POST /auth/resend-verification`
- Backend adapter hook: `POST /auth/outbox` for `verify_email` and `password_reset` delivery events.
- Multipart field parsing support on `POST /forms/submit` (`multipart/form-data`, text fields).
- New auth config knobs:
  - `SSMA_REFRESH_COOKIE`
  - `SSMA_AUTH_REFRESH_ENABLED`
  - `SSMA_REFRESH_TTL_MS`
  - `SSMA_AUTH_REQUIRE_EMAIL_VERIFICATION`
  - `SSMA_EMAIL_VERIFY_TTL_MS`
  - `SSMA_PASSWORD_RESET_TTL_MS`
- E2E coverage for refresh, forgot/reset password, email verify/resend, and multipart forms.

### Changed

- Auth responses now consistently include `x-request-id` headers.
- Register/login/OIDC flows now issue refresh cookies when refresh flow is enabled.
- Logout and password reset flows clear refresh cookies/tokens.

## [0.4.0] - 2026-04-08

### Added

- Webhook ingress route: `POST /webhooks/:provider`
- Webhook verification modes: `disabled` and `external`
- Webhook idempotency window with in-memory dedupe (`provider:eventId`)
- Backend webhook adapter forwarding: `POST /webhooks/ingest`
- OIDC bridge routes:
  - `GET /auth/oidc/start`
  - `GET /auth/oidc/callback`
- Request-id handling:
  - accept/generate `x-request-id`
  - forward request id to backend adapter calls
- New E2E coverage:
  - webhook flows (`e2e_webhooks.rs`)
  - OIDC bridge (`e2e_oidc.rs`)
  - urlencoded form + CSRF (`e2e_forms.rs`)

### Changed

- Form ingress now accepts `application/x-www-form-urlencoded` in addition to JSON.
- Added optional CSRF double-submit enforcement for urlencoded form submissions.
- Switched media and internal asset content responses to streaming file reads.
- Added compression and HTTP tracing middleware in transport stack.
- Added stricter route-level payload controls for forms/webhooks/query families.
- Expanded deployment docs to make HTTP/2 and HTTP/3 edge-termination guidance explicit.

## [0.3.0] - 2026-04-08

### Added

- Core form-handling route: `POST /forms/submit`
- Generic anti-bot handling in gateway core:
  - honeypot hook (silent-drop behavior via `202 accepted`)
  - captcha verification hook modes (`disabled` and `external`)
  - dedicated form-rate limiting
- Backend adapter support for form forwarding:
  - new backend contract endpoint: `POST /forms/submit`
- E2E test coverage for form handling:
  - valid forward path
  - honeypot drop path
  - external captcha pass/fail/timeout
  - form rate-limit enforcement
  - invalid payload rejection

### Changed

- Updated template docs, config docs, and backend contract docs to include form handling.

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
