# SSMA Roadmap

Split the work into two tracks:

1. `examples/` for later reference implementations
2. SSMA template hardening for single-node production readiness

This keeps SSMA core focused as a gateway template while still planning a full reference stack.

## Track A — Future `examples/`

These are intentionally **out of SSMA core**.

Status:
- [ ] `examples/sqlite-backend/`
- [ ] `examples/admin-ui/`
- [ ] end-to-end local startup docs for examples
- [ ] example integration smoke test

### `examples/sqlite-backend/`

Purpose:
- reference backend implementing the SSMA adapter contract
- single-node durable business backend with SQLite
- concrete example for `apply-intents`, `query`, `subscribe`, `health`

Scope:
- SQLite schema for:
  - users
  - roles
  - one simple domain model such as `todos` or `tasks`
  - audit log
- endpoints:
  - `POST /apply-intents`
  - `POST /query/:name`
  - `POST /subscribe`
  - `POST /health`
- one real optimistic workflow:
  - create/update/delete records
  - backend emits invalidations as needed
- seed script:
  - admin user
  - sample users
  - sample domain records

Recommended later additions:
- backend auth/session for admin UI
- audit trail viewer queries
- role-aware query examples

### `examples/admin-ui/`

Purpose:
- reference admin surface for operating the example backend + SSMA
- show how a real app consumes gateway/admin/backend APIs

Scope:
- login page
- admin session auth
- dashboard:
  - gateway health
  - backend health
  - basic counts
- users page:
  - list users
  - create user
  - disable/enable user
  - assign role
- intents page:
  - pending intents
  - recent optimistic activity
- channels page:
  - active subscriptions from SSMA admin API
- logs page:
  - recent audit log
  - recent gateway events if exposed

Recommended role model:
- `user`
- `staff`
- `admin`

Explicit non-goals for first version:
- full workflow engine UI
- multi-tenant management
- advanced analytics
- media/WebRTC admin

### Example Integration Later

After both examples exist:
- document end-to-end local startup
- add one integration test or smoke script
- make examples clearly labeled as reference implementations, not SSMA core

Track A owns concrete persistence examples such as SQLite-backed business backends.

## Track B — SSMA Template Tier 2

These are the items that **do belong in SSMA core** for single-node production readiness.

Status:
- [x] `/ready` endpoint
- [x] startup config validation
- [x] backend timeout policy
- [x] cookie hardening basics (`HttpOnly`, `Secure`, `SameSite`)
- [x] WS backpressure handling
- [x] graceful shutdown handling
- [x] tests for health/readiness, backpressure, shutdown, streaming
- [x] structured server-event logging cleanup for current runtime
- [x] single-node deployment docs polish
- [x] stricter readiness edge-case hardening pass

### 1. Readiness and Startup Hardening

Checklist:
- [x] add `/ready` endpoint
- [x] fail fast on invalid config at boot
- [x] validate required secrets
- [x] validate path writability
- [x] validate cookie/security config consistency
- [x] validate malformed origin lists

Why:
- production deployment needs fast operator feedback
- health alone is not enough

### 2. Explicit Timeout Policy

Checklist:
- [x] explicit backend HTTP client timeouts
- [x] explicit request timeout policy for long-lived vs short-lived routes
- [x] documented timeout defaults in config docs

Why:
- single-node production still needs bounded failure behavior

### 3. Logging and Observability Hardening

Checklist:
- [x] standardize structured log fields for current `emit_server_event()` path
- [x] standardize event names via uppercase server event counters
- [x] document operator-facing logs enough for current runtime
- [x] ensure important failures emit consistent structured events

Why:
- current metrics/events are useful but still uneven

### 4. Security and Cookie Hardening

Checklist:
- [x] tighten `Secure` cookie behavior
- [x] tighten `HttpOnly` cookie behavior
- [x] expose `SameSite` config
- [x] add proxy-aware deployment notes
- [ ] document same-origin vs cross-origin behavior more explicitly

Why:
- current auth works, but production operators need predictable cookie semantics

### 5. Persistence Story Boundaries

Checklist:
- [x] keep SSMA core generic
- [x] document current built-in persistence assumptions clearly
- [x] leave opinionated database-backed examples to Track A

Current direction:
- SSMA core keeps file-backed intent/user persistence for template simplicity
- Track A may provide SQLite-backed backend examples for operators who want a concrete durable stack

### 6. Error Model and Operator Clarity

Checklist:
- [ ] normalize gateway error responses where still inconsistent
- [x] ensure internal logs explain operator-visible failures
- [ ] document stable error codes more explicitly

Why:
- production support depends on predictable failure surfaces

### 7. Backpressure and Connection Hardening

Checklist:
- [x] review WS backpressure behavior and document limits
- [ ] verify SSE slow-consumer handling expectations more explicitly
- [x] review reconnect and shutdown edge cases
- [x] add missing tests for connection lifecycle edge cases

Why:
- this is gateway-critical behavior

### 8. Documentation Cleanup

Checklist:
- [x] keep docs aligned with active runtime only
- [x] separate active gateway features from deferred features
- [x] separate active gateway features from example-stack features
- [x] add dev guidance
- [x] add single-node production guidance

Why:
- template credibility depends on tight scope and accurate docs

## Strict Tier 2 Priority Order

Implemented order:

1. [x] `/ready` + startup validation
2. [x] backend timeout policy
3. [x] cookie/security hardening
4. [x] structured logging cleanup
5. [x] edge-case tests for shutdown/reconnect/backpressure
6. [x] deployment documentation

## Boundary Rule

Keep this rule strict:

- SSMA core owns gateway concerns
- `examples/` own business logic examples and admin UI
- `examples/` own opinionated persistence stacks such as SQLite
- durable business workflows stay out of SSMA core
- media/WebRTC stay out of SSMA core unless explicitly reintroduced as product scope
