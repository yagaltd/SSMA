---
name: ssma-security
description: Modify auth, RBAC, rate limiting, or CORS. Use when changing transport/auth.rs, security config, or access control logic.
---

# SSMA Security

## Authentication

Session cookie based auth using `ssma_session` (configurable via `SSMA_AUTH_COOKIE`).

### Auth Flow

1. **Register** → `POST /auth/register` → Argon2id hash → store in JSON file → issue JWT → set cookie
2. **Login** → `POST /auth/login` → verify password → issue JWT → set cookie
3. **Logout** → `POST /auth/logout` → clear cookie
4. **Me** → `GET /auth/me` → decode JWT → return user profile

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
| Channel subscribe | `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` / `SSMA_OPTIMISTIC_CHANNEL_MAX` | 8 / 10s |
| Rework/undo | `SSMA_OPTIMISTIC_REWORK_WINDOW_MS` / `SSMA_OPTIMISTIC_REWORK_MAX` | 20 / 60s |
| WS backpressure | `SSMA_WS_MAX_BUFFERED_BYTES` | 256 KB |

Rate-limited responses:
- HTTP: `429 Too Many Requests` with `RATE_LIMITED` error code
- WS: `{ type: "error", code: "RATE_LIMITED" }`
- Channel subscribe: `{ type: "channel.ack", status: "error", code: "RATE_LIMITED" }`

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
