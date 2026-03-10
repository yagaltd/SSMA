# SSMA Documentation Hub

This documentation covers both SSMA runtimes:
- JS runtime: `apps/ssma-js`
- Rust runtime: `apps/ssma-rust`

Shared protocol assets live in:
- `packages/ssma-protocol/contracts`
- `packages/ssma-protocol/vectors`

## Start Here

If you are new to the repo:
- `index.md`
- `guides/SSMA-IN-A-NUTSHELL.md`
- `guides/SSMA-RUNTIME.md`

If you are changing behavior:
- `architecture/backend-interface.md`
- `protocol/wire-protocol.md`
- `security/auth.md`
- `security/rbac.md`

## Documentation Map

- Architecture:
  - `architecture/gateway-overview.md`
  - `architecture/modules.md`
  - `architecture/backend-interface.md`
- API:
  - `api/http-endpoints.md`
  - `api/streaming.md`
  - `api/admin.md`
- Security:
  - `security/auth.md`
  - `security/rbac.md`
  - `security/rate-limits.md`
- Operations:
  - `operations/config.md`
  - `operations/deployment.md`
- Testing:
  - `testing/conformance.md`
  - `testing/e2e-harness.md`
- Guides:
  - `guides/SSMA-IN-A-NUTSHELL.md`
  - `guides/SSMA-RUNTIME.md`
  - `guides/SSMA-OPTIMISTIC-SYNC.md`
- Roadmaps:
  - `../roadmap.md`
  - `roadmap/rust-parity-checklist.md`
  - `roadmap/SSMA-FUTURE-ENHANCEMENTS.md`

## Maintenance Rule

When runtime behavior changes, update the relevant contract docs first and then update supporting guides.
