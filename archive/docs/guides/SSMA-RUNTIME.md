# SSMA Runtime & Architecture

This guide explains the shared runtime model across JS and Rust implementations and where each runtime differs.

## Layered Overview

1. **Protocol Layer**: contracts and vectors from `packages/ssma-protocol`.
2. **Transport Layer**: HTTP health/metrics + WS/SSE sync endpoints.
3. **State Layer**: intent store, cursor/log sequence, replay window, dedupe.
4. **Policy Layer**: schema validation, auth extraction, RBAC, rate limits.
5. **Adapter Layer**: backend interface (`applyIntents`, `query`, `subscribe`, `health`).
6. **Observability Layer**: structured server events and transport metrics.

## Runtime Mapping

| Concern | JS Runtime | Rust Runtime |
| --- | --- | --- |
| Runtime root | `apps/ssma-js` | `apps/ssma-rust` |
| Protocol loading | `src/runtime/ContractRegistry.js` | `src/protocol.rs` |
| Gateway transport | `services/optimistic/SyncGateway.js` | `src/gateway.rs` |
| Intent persistence | `services/optimistic/IntentStore.js` | `src/runtime.rs` |
| Backend adapter | `src/backend/BackendHttpClient.js` | `src/backend.rs` |

## Shared Protocol and Validation

- Contracts live in `packages/ssma-protocol/contracts`.
- Vectors live in `packages/ssma-protocol/vectors`.
- Both runtimes validate inbound frames before dispatch.
- Both runtimes replay shared vectors in conformance tests.

## Gateway Pipeline

Canonical processing order:

1. Validate inbound frame contract.
2. Enforce role/auth/rate-limit policy.
3. Persist + dedupe intents.
4. Forward fresh intents to backend.
5. Normalize result statuses.
6. Emit ACK + invalidate/island.invalidate fanout.

## Runtime-Specific Notes

- JS runtime includes additional auth/log routes and services.
- Rust runtime currently focuses on optimistic gateway and protocol parity.

## Extending the System

1. Add or update contracts in `packages/ssma-protocol/contracts`.
2. Add vectors in `packages/ssma-protocol/vectors`.
3. Implement behavior in both runtimes.
4. Update conformance and E2E tests.
5. Update docs in this directory.
