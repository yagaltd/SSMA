# SSMA Ecosystem Roadmap

## Decision Summary

This roadmap turns three decisions into execution work:

1. `ssma-backend-starter` is worth building, but it must remain an optional ecosystem package.
2. Tauri support is worth pursuing, but only as a lightweight integration path first.
3. CSMA should not get a new Tauri transport module unless the architecture moves away from embedded HTTP/WebSocket and commits to Tauri IPC.

The goal is to reduce integration friction without weakening the core SSMA contract or creating a second unsupported platform surface too early.

---

## Product Boundaries

### SSMA Core

SSMA core owns:
- protocol contracts
- JS and Rust runtimes
- auth, RBAC, lifecycle, replay, invalidation semantics
- backend adapter contract

SSMA core does not own:
- Stripe/SES/S3 product decisions
- framework-specific backend scaffolding
- Tauri-native IPC transport

### Ecosystem Addons

Ecosystem addons may own:
- backend starter templates and adapters
- Tauri integration templates
- Tauri plugin experiments
- framework-specific examples

Rule:
- addon packages must follow the core contract
- addon packages must not redefine the contract

---

## Track A: `ssma-backend-starter`

### Objective

Ship a practical backend accelerator for teams adopting SSMA, without turning SSMA itself into an opinionated backend framework.

### Positioning

`ssma-backend-starter` should be:
- optional
- copy-paste friendly
- contract-aligned
- opinionated in implementation, not in SSMA semantics

It should not be required to use SSMA.

### Phase 1: Stabilize the Contract

Tasks:
- freeze the canonical backend context shape: `{ site, connectionId, ip, userAgent, user }`
- document the starter as an adapter helper, not as part of SSMA core
- define the minimum starter promise: every example must match the official backend contract docs

Deliverables:
- starter proposal doc aligned with the canonical context
- clear README language that says optional package/template

Acceptance criteria:
- no starter example uses snake_case adapter context fields
- no starter example assumes direct cookie parsing in backend adapters

### Phase 2: Ship a Minimal Starter

Tasks:
- create a minimal Node starter with `apply-intents`, `query`, `subscribe`, and `health`
- provide one router helper for common Node stacks
- provide one validation/error helper layer
- include one tiny real example adapter, not five

Recommended first adapter:
- email or storage before payments

Reason:
- payment adapters add business and compliance assumptions too early

Deliverables:
- one minimal starter package or template
- one example backend app using the canonical SSMA contract

Acceptance criteria:
- a new developer can connect SSMA to a backend in under 30 minutes
- the starter passes a small E2E example against `ssma-js` and `ssma-rust`

### Phase 3: Expand Carefully

Tasks:
- add adapters only after real demand appears
- prioritize adapters by repeated usage, not by theoretical completeness
- keep each adapter isolated so teams can copy only what they need

Good candidates later:
- Resend
- S3 / R2
- Stripe

Do not do now:
- giant all-in-one backend framework
- mandatory runtime dependency from SSMA core to starter

---

## Track B: Tauri Support

### Objective

Make SSMA usable in Tauri apps quickly, without committing to a second runtime model before demand is proven.

### Positioning

Tauri support should begin as:
- integration guidance
- template scaffolding
- embedded-runtime usage

It should not begin as:
- a new SSMA runtime
- a new protocol
- a CSMA-only fork

### Phase 1: Embedded HTTP/WebSocket Template

Tasks:
- create a Tauri example that embeds the existing Rust SSMA gateway
- wire app startup and shutdown to SSMA lifecycle
- expose the local endpoint to the frontend config
- document desktop-first constraints clearly

Deliverables:
- Tauri example app or template
- startup/shutdown lifecycle notes
- local development instructions

Acceptance criteria:
- a Tauri app can boot SSMA, connect from CSMA, and shut down cleanly
- the existing HTTP/WebSocket contract remains unchanged

### Phase 2: Harden the Integration

Tasks:
- verify window lifecycle and teardown across multiple windows
- document how auth cookies/session tokens behave in embedded Tauri flows
- document platform limits for desktop vs mobile packaging
- decide whether this path is desktop-only or desktop+mobile

Deliverables:
- verified lifecycle notes
- packaging guidance
- known-limits section

Acceptance criteria:
- no leaked reconnect loops or orphaned gateway processes on app close
- one documented recommended setup for desktop

### Phase 3: Plugin Evaluation

Tasks:
- evaluate whether a `tauri-plugin-ssma` brings real value beyond the template
- measure whether plugin packaging reduces duplication enough to justify maintenance
- keep transport semantics identical to the existing Rust gateway

Build only if:
- multiple apps need the same embedded setup
- the template proves repetitive

Do not do now:
- full Tauri-native IPC rewrite
- separate Tauri-specific wire contract

---

## Track C: CSMA + Tauri

### Objective

Support Tauri users quickly without forcing a premature CSMA transport rewrite.

### Default Strategy

Use the existing CSMA optimistic transport against embedded SSMA over local HTTP/WebSocket.

That means:
- CSMA mostly needs a Tauri app template
- CSMA does not need a new sync module yet
- SSMA remains the same gateway contract

### Phase 1: Template Support

Tasks:
- add a Tauri-oriented CSMA template or setup guide
- document how endpoint discovery/config works in Tauri
- confirm channel replay and invalidation work unchanged

Deliverables:
- CSMA Tauri template
- one SSMA + CSMA + Tauri example path

Acceptance criteria:
- a developer can launch a Tauri shell with CSMA frontend and embedded SSMA backend
- no transport code changes are required inside CSMA core

### Phase 2: Decide on IPC Later

Only consider a new CSMA transport module if all of this becomes true:
- Tauri becomes a strategic target
- embedded HTTP/WebSocket is measurably inadequate
- the team commits to a Tauri IPC contract as a product surface

If those conditions are met:
- design a dedicated transport abstraction first
- then build CSMA Tauri IPC support
- then evaluate whether SSMA needs a true Tauri-native runtime

Until then:
- no new CSMA transport module

---

## Sequence

### Build Now

1. Keep `ssma-backend-starter` as an optional package/template.
2. Build a minimal starter, not a full backend framework.
3. Build a desktop-first Tauri integration template using embedded `ssma-rust`.
4. Add a CSMA Tauri template/config path that reuses the current HTTP/WebSocket transport.

### Build Later

1. Additional starter adapters after real usage signals.
2. A reusable Tauri plugin if the template becomes repetitive.
3. Mobile-oriented Tauri guidance after desktop is stable.

### Do Not Build Now

1. A new SSMA Tauri runtime.
2. A new CSMA Tauri transport module.
3. A mandatory backend starter dependency in SSMA core.

---

## Success Metrics

### Backend Starter

- first backend integration time drops materially
- fewer contract-shape mistakes in backend adapters
- starter examples work unchanged against both `ssma-js` and `ssma-rust`

### Tauri

- one documented Tauri path works end-to-end
- startup and shutdown are clean
- no new protocol surface is introduced

### CSMA

- no forked transport logic
- existing optimistic-sync behavior works unchanged in the embedded setup

---

## Exit Criteria for Reassessment

Revisit the strategy if any of these happens:
- multiple teams ask for Tauri-native IPC instead of embedded HTTP/WebSocket
- mobile Tauri becomes a real delivery target
- the starter package grows into a framework and starts distorting SSMA core decisions
- maintenance cost of templates exceeds the cost of packaging them as supported addons

---

## Final Recommendation

Build the ecosystem around SSMA, not inside SSMA.

That means:
- backend starter: yes, optional, minimal, contract-first
- Tauri support: yes, template first, plugin later if justified
- CSMA Tauri module: no, not until IPC becomes a deliberate architectural commitment
