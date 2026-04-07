# SSMA-JS

Node.js implementation of the SSMA gateway runtime.

## What It Does

- Exposes HTTP APIs, WebSocket sync, and SSE invalidation streams.
- Validates payloads against shared contracts from `packages/ssma-protocol/contracts`.
- Enforces auth/RBAC/rate-limit policies.
- Persists optimistic intent state and emits transport events.

## Directory Map

- `src/`: runtime, routes, services, backend adapter, contract registry
- `tests/`: unit, conformance, and E2E harness tests
- `scripts/`: sync, docs/security checks, local tooling
- `examples/`: local simulation helpers
- `data/`: local JSON stores used in development

## Environment

This app loads environment variables from `apps/ssma-js/.env` via `dotenv`.

1. Copy `.env.example` to `.env`.
2. Set secrets and URLs for your environment.

Common variables:

- `SSMA_PORT`
- `SSMA_PROTOCOL_SUBPROTOCOL`
- `SSMA_BACKEND_URL`
- `SSMA_BACKEND_INTERNAL_TOKEN`
- `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES`

## Run

From repo root:

- `npm --prefix apps/ssma-js run dev`
- `npm --prefix apps/ssma-js run start`

From `apps/ssma-js`:

- `npm run dev`
- `npm run start`

## Test

From repo root:

- `npm --prefix apps/ssma-js run test`
- `npm --prefix apps/ssma-js run test:conformance`
- `npm --prefix apps/ssma-js run check:docs`

