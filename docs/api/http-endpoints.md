# HTTP Endpoints

## Shared (JS + Rust)

- `GET /health`
- `GET /optimistic/events`
- `GET /optimistic/metrics`

## Internal (JS + Rust)

- `POST /internal/backend/events`

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
