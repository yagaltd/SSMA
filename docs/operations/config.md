# Config

## Shared Env Variables

- `SSMA_PROTOCOL_SUBPROTOCOL`
- `SSMA_BACKEND_URL`
- `SSMA_BACKEND_INTERNAL_TOKEN`
- `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES`
- `SSMA_OPTIMISTIC_REPLAY_MS`
- `SSMA_RATE_WINDOW_MS`
- `SSMA_RATE_MAX`
- `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS`
- `SSMA_OPTIMISTIC_CHANNEL_MAX`

## JS Runtime (`apps/ssma-js`)

- Main env file: `apps/ssma-js/.env`
- Template: `apps/ssma-js/.env.example`
- Additional JS settings:
  - `SSMA_BACKEND_TIMEOUT_MS`
  - auth/logging related settings used by JS-only routes.

## Rust Runtime (`apps/ssma-rust`)

- Rust reads process environment directly.
- Template: `apps/ssma-rust/.env.example`
- Additional Rust settings:
  - `SSMA_OPTIMISTIC_STORE`
  - `SSMA_OPTIMISTIC_PROTECTED_CHANNELS`
  - `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE`
