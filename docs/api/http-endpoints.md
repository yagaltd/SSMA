# HTTP Endpoints

## Shared (JS + Rust)

- `GET /health`
- `GET /optimistic/events`
- `GET /optimistic/metrics`

## Rust Runtime Session A (`apps/ssma-rust`)

- `POST /query/:name`
- `POST /media/assets`
- `GET /media/assets/:assetId`
- `GET /media/assets/:assetId/content`
- `DELETE /media/assets/:assetId`
- `POST /rtc/sessions`
- `POST /rtc/sessions/:sessionId/signals`

## Internal (JS + Rust)

- `POST /internal/backend/events`

## Internal Adapter Access (Rust Session A)

- `POST /internal/assets`
- `GET /internal/assets/:assetId`
- `GET /internal/assets/:assetId/content`
- `DELETE /internal/assets/:assetId`

Notes:
- `/query/:name` is the public request/response gateway path for one-shot backend queries.
- `/media/*` is the public asset ingress/egress boundary and is owned by SSMA itself.
- `/internal/assets/*` is backend-only and is protected by `x-ssma-backend-token`.
- Guest-owned assets are bound to the anonymous session cookie (`SSMA_ANON_COOKIE`, default `ssma_anon`).

## JS Runtime Only (`apps/ssma-js`)

- `POST /auth/register`
- `POST /auth/login`
- `POST /auth/logout`
- `GET /auth/me`
- `POST /auth/refresh`
- `POST /auth/api-key/issue`
- `POST /auth/api-key/login`
- `POST /auth/hmac/nonce`
- `POST /logs/batch`
- `GET /logs/health`
