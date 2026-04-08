---
name: ssma-config
description: Configure, deploy, or operate the SSMA gateway. Use when modifying config.rs, .env.example, or deployment settings.
---

# SSMA Configuration & Operations

## Environment Variables

All config is loaded from environment variables in `config.rs` → `Config::from_env()`.

### Server

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_HOST` | `127.0.0.1` | Bind address |
| `SSMA_PORT` | `5050` | Listen port |
| `SSMA_PROTOCOL_SUBPROTOCOL` | `1.0.0` | Protocol version |

### Backend Adapter

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_BACKEND_URL` | (empty) | Backend base URL |
| `SSMA_BACKEND_INTERNAL_TOKEN` | (empty) | Token for `/internal/*` routes |
| `SSMA_BACKEND_TIMEOUT_MS` | `5000` | Backend request timeout |

### Authentication

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_AUTH_JWT_SECRET` | ⚠️ insecure | JWT signing secret (**MUST** set in prod) |
| `SSMA_AUTH_COOKIE` | `ssma_session` | Session cookie name |
| `SSMA_ANON_COOKIE` | `ssma_anon` | Anonymous session cookie name |
| `SSMA_JWT_ISSUER` | `ssma-auth-service` | JWT issuer claim |
| `SSMA_JWT_AUDIENCE` | `csma-clients` | JWT audience claim |
| `SSMA_ACCESS_TTL_MS` | `900000` | JWT lifetime (15 min) |
| `SSMA_AUTH_COOKIE_SECURE` | `true` | Set `Secure` flag on cookies |
| `SSMA_AUTH_COOKIE_SAMESITE` | `Lax` | Cookie `SameSite` mode |

### CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_ALLOWED_ORIGINS` | (empty) | Comma-separated origins, `*` for all, empty = no CORS |

### Rate Limiting

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_RATE_WINDOW_MS` | `60000` | Global rate window |
| `SSMA_RATE_MAX` | `120` | Global max requests per window |
| `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` | `10000` | Channel subscribe burst window |
| `SSMA_OPTIMISTIC_CHANNEL_MAX` | `8` | Max subscribes per burst |
| `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` | `60000` | Rework rate window |
| `SSMA_OPTIMISTIC_REWORK_MAX` | `20` | Max reworks per window |

### Optimistic Store

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_OPTIMISTIC_STORE` | `./data/optimistic-intents-rust.json` | Intent persistence file |
| `SSMA_OPTIMISTIC_REPLAY_MS` | `300000` | Replay window (5 min) |
| `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES` | `false` | Require auth for intent.batch |
| `SSMA_OPTIMISTIC_PROTECTED_CHANNELS` | (empty) | Comma-separated protected channel names |
| `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE` | `admin` | Min role for protected channels |
| `SSMA_OPTIMISTIC_MAX_ENTRIES` | `5000` | Max entries before trimming |

### User Store

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_USER_STORE` | `./data/users.json` | User persistence file |

### Media

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_MEDIA_STORAGE_ROOT` | `./data/media` | Asset file storage directory |
| `SSMA_MEDIA_MAX_UPLOAD_BYTES` | `52428800` | Max upload size (50 MB) |
| `SSMA_MEDIA_TTL_SECS` | `3600` | Asset lifetime (1 hour) |

### Transport

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_SSE_RETRY_MS` | `2500` | SSE retry/keep-alive interval |
| `SSMA_WS_MAX_BUFFERED_BYTES` | `262144` | WS backpressure threshold (256 KB) |

### Logging

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_LOG_RELAY_URL` | (empty) | Log forwarding endpoint (empty = disabled) |

### Forms

| Variable | Default | Description |
|----------|---------|-------------|
| `SSMA_FORM_RATE_WINDOW_MS` | `60000` | Form submission rate window |
| `SSMA_FORM_RATE_MAX` | `20` | Max form submissions per window (per site+IP bucket) |
| `SSMA_FORM_CAPTCHA_MODE` | `disabled` | Captcha mode: `disabled` or `external` |
| `SSMA_FORM_CAPTCHA_VERIFY_URL` | (empty) | External verifier URL (required when mode is `external`) |
| `SSMA_FORM_CAPTCHA_TIMEOUT_MS` | `3000` | External verifier timeout |

## Deployment

### Single Instance

```bash
cd apps/ssma-rust
cargo run --release
```

Recommended for real single-node deployments:
- set `SSMA_AUTH_JWT_SECRET` explicitly
- keep persistent storage mounted for `./data`
- run behind HTTPS reverse proxy
- keep `SSMA_AUTH_COOKIE_SECURE=true`
- use `/ready` for readiness probes
- use `/health` for simple liveness checks

### Docker

```dockerfile
FROM rust:1.75 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/ssma-rust /usr/local/bin/
EXPOSE 5050
CMD ["ssma-rust"]
```

### Behind Reverse Proxy

- Ensure proxy supports WebSocket upgrade (`Upgrade: websocket`)
- Ensure proxy supports SSE streaming (disable buffering)
- Pass client IP via `X-Forwarded-For` header

### Horizontal Scaling Notes

- In-memory rate limits don't sync across instances → use Redis
- Intent store is file-based → use shared storage (NFS, S3, Postgres)
- Channel registry is in-memory → use Redis pub/sub for cross-instance fanout
- User store is file-based → use shared database

## Health Check

```bash
curl http://localhost:5050/health
```

Response:
```json
{
  "status": "ok",
  "service": "ssma-rust",
  "subprotocol": "1.0.0",
  "cursor": 12345
}
```

## Readiness Check

```bash
curl http://localhost:5050/ready
```

Readiness semantics:
- `200 ok` when gateway is booted and backend is either healthy or unconfigured
- `503 not_ready` when configured backend is unreachable

Use `/ready` for orchestrators. Use `/health` for lightweight liveness.

## Metrics

```bash
curl http://localhost:5050/optimistic/metrics
```

## Data Files

| Path | Purpose |
|------|---------|
| `./data/optimistic-intents-rust.json` | Intent store (auto-created) |
| `./data/users.json` | User store (auto-created) |
| `./data/media/` | Uploaded assets (auto-created, cleaned on startup) |
