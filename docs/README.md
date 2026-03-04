# SSMA Documentation Hub

Server-Side Microservices Architecture (SSMA) is a gateway-first backend that is implemented in both JS and Rust in this repository.

Repository mapping:

- JS runtime: `apps/ssma-js`
- Rust runtime: `apps/ssma-rust`
- Shared protocol contracts and vectors: `packages/ssma-protocol`

## Guide Map

| Goal | Start Here | Summary |
| --- | --- | --- |
| **Need a 5-minute refresher** | [`guides/SSMA-IN-A-NUTSHELL.md`](./guides/SSMA-IN-A-NUTSHELL.md) | Cross-runtime overview of gateway responsibilities, flow, and boundaries. |
| **Understand runtime internals** | [`guides/SSMA-RUNTIME.md`](./guides/SSMA-RUNTIME.md) | Runtime architecture for both implementations with JS/Rust mapping. |
| **Operate or deploy the backend** | [`operations/config.md`](./operations/config.md) + [`operations/deployment.md`](./operations/deployment.md) | Environment variables, runtime constraints, deployment targets, and ops notes. |
| **Review security posture** | [`security/auth.md`](./security/auth.md) + [`security/rate-limits.md`](./security/rate-limits.md) + [`security/rbac.md`](./security/rbac.md) | Auth model, RBAC policy, and rate-limit controls. |
| **Track upcoming work** | [`roadmap/SSMA-FUTURE-ENHANCEMENTS.md`](./roadmap/SSMA-FUTURE-ENHANCEMENTS.md) + [`roadmap/rust-parity-checklist.md`](./roadmap/rust-parity-checklist.md) | Architectural evolution and Rust parity milestones. |

## Runtime Capability Matrix

| Capability | JS (`apps/ssma-js`) | Rust (`apps/ssma-rust`) |
| --- | --- | --- |
| WS `/optimistic/ws` | Yes | Yes |
| SSE `/optimistic/events` | Yes | Yes |
| Backend ingest `/internal/backend/events` | Yes | Yes |
| Metrics endpoint | Yes (`/optimistic/metrics`) | Yes (`/optimistic/metrics`) |
| Shared schema validation | Yes | Yes |
| Shared vector conformance tests | Yes | Yes |

Canonical schemas and vectors are defined in:

- `../packages/ssma-protocol/contracts`
- `../packages/ssma-protocol/vectors`
