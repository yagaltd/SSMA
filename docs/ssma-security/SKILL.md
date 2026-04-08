---
name: ssma-security
description: Modify auth, RBAC, rate limiting, or CORS. Use when changing transport/auth.rs, security config, or access control logic.
---

# SSMA Security

## Authentication

Session cookie based auth using `ssma_session` (configurable via `SSMA_AUTH_COOKIE`).

### Auth Flow

1. **Register** → `POST /auth/register` → Argon2id hash → store in JSON file → issue session + refresh cookies
2. **Login** → `POST /auth/login` → verify password (and optional email verification gate) → issue cookies
3. **Refresh** → `POST /auth/refresh` → rotate refresh token + issue new session cookie
4. **Logout** → `POST /auth/logout` → clear session and refresh cookies
5. **Me** → `GET /auth/me` → decode JWT → return user profile
6. **Recovery** → `POST /auth/forgot-password` + `POST /auth/reset-password`
7. **Email verification** → `POST /auth/verify-email` + `POST /auth/resend-verification`

### Response Shape

All auth endpoints return `{ status: "ok", user: {...} }`:

```json
{
  "status": "ok",
  "user": {
    "id": "uuid",
    "email": "user@example.com",
    "name": "User",
    "role": "user",
    "status": "active",
    "emailVerified": true,
    "createdAt": 1234567890,
    "updatedAt": 1234567890,
    "lastLoginAt": null
  }
}
```

This envelope matches CSMA's `AuthService` expectation of `response.user`.

### JWT Structure

- Algorithm: HS256
- Claims: `sub` (user ID), `role`, `iss`, `aud`, `iat`, `exp`
- Secret: `SSMA_AUTH_JWT_SECRET` (**must** be set in production)
- TTL: `SSMA_ACCESS_TTL_MS` (default 15 minutes)

### Cookie Settings

- `HttpOnly` — not accessible to JavaScript
- `SameSite=Lax` — CSRF protection
- `Secure` — only sent over HTTPS (controlled by `SSMA_AUTH_COOKIE_SECURE`, default `true`)
- `Path=/`
- Refresh cookie name: `SSMA_REFRESH_COOKIE` (default `ssma_refresh`)

Auth endpoints also include `x-request-id` for traceability.

### WS/SSE Auth

- WebSocket: parses `Cookie` header during upgrade, decodes JWT, extracts role
- SSE: same cookie parsing
- Missing/invalid token → `authRole: "guest"`

## RBAC

Role hierarchy (rank):

| Role | Rank |
|------|------|
| guest | 0 |
| user | 1 |
| staff | 2 |
| admin | 3 |
| system | 4 |

### Protected Channels

Config: `SSMA_OPTIMISTIC_PROTECTED_CHANNELS` (comma-separated channel names)
Min role: `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE` (default `admin`)

When a connection with insufficient role subscribes to a protected channel:
- Emit `CHANNEL_ACCESS_DENIED` server event
- Send `channel.close` frame to the client

### Island Invalidation RBAC

Island access is configured in `default_island_access()` in `config.rs`:

```rust
HashMap::from([
    ("product-inventory", "guest"),
    ("product-reviews", "user"),
    ("blog-comments", "user"),
    ("ops.dashboard", "staff"),
])
```

SSE and WS both enforce island RBAC before fanout.

### Write Auth

Controlled by `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES` (default `false`).

When `true`, `intent.batch` from unauthenticated connections returns `UNAUTHORIZED`.

## Rate Limiting

All rate limits use in-memory `Mutex<HashMap<String, RateBucket>>`. Acceptable for single-instance; use Redis for horizontal scale.

| Limiter | Config | Default |
|---------|--------|---------|
| Global HTTP | `SSMA_RATE_WINDOW_MS` / `SSMA_RATE_MAX` | 120 req / 60s |
| Forms submit | `SSMA_FORM_RATE_WINDOW_MS` / `SSMA_FORM_RATE_MAX` | 20 req / 60s |
| Channel subscribe | `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` / `SSMA_OPTIMISTIC_CHANNEL_MAX` | 8 / 10s |
| Rework/undo | `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` / `SSMA_OPTIMISTIC_REWORK_MAX` | 20 / 60s |
| WS backpressure | `SSMA_WS_MAX_BUFFERED_BYTES` | 256 KB |

Rate-limited responses:
- HTTP: `429 Too Many Requests` with `RATE_LIMITED` error code
- WS: `{ type: "error", code: "RATE_LIMITED" }`
- Channel subscribe: `{ type: "channel.ack", status: "error", code: "RATE_LIMITED" }`

## Form Anti-Bot Policy

Route: `POST /forms/submit`

- Honeypot:
  - non-empty honeypot field returns `202 accepted`
  - request is dropped and not forwarded to backend
- Captcha:
  - `SSMA_FORM_CAPTCHA_MODE=disabled` bypasses captcha verification
  - `SSMA_FORM_CAPTCHA_MODE=external` requires `captchaToken`
  - failed/unreachable verifier is fail-closed (`CAPTCHA_VERIFICATION_FAILED`)

## Webhook Security Policy

Route: `POST /webhooks/:provider`

- Verification modes:
  - `SSMA_WEBHOOK_VERIFY_MODE=disabled`: webhook payload must include `eventId` and `eventType`
  - `SSMA_WEBHOOK_VERIFY_MODE=external`: webhook is validated by external verifier service
- Idempotency:
  - event keys are tracked in-memory for `SSMA_WEBHOOK_IDEMPOTENCY_TTL_SECS`
  - duplicate events are accepted but not re-forwarded
- Request tracing:
  - `x-request-id` is accepted/generated and forwarded to backend

## CORS

Config: `SSMA_ALLOWED_ORIGINS`

- Empty (default): no CORS headers sent
- `*`: allow all origins
- Comma-separated list: `https://example.com,https://app.example.com`

Implementation: `tower-http` CorsLayer

## Key Files

- `transport/auth.rs` — UserStore, register/login/me handlers, password hashing
- `transport/mod.rs` — `resolve_actor_from_headers()`, `role_rank()`, rate limiters, CORS
- `config.rs` — All `SSMA_AUTH_*`, `SSMA_RATE_*`, `SSMA_ALLOWED_ORIGINS`

## Rules

- Never log JWT secrets or password hashes
- Always use Argon2id for password hashing
- JWT must validate `exp` in production (tests may disable for convenience)
- Guest connections are valid — don't reject them, just limit capabilities
- Internal routes (`/internal/*`) require `x-ssma-backend-token`
