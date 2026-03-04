# SSMA API Documentation

This API powers the Simple Security & Monitoring Agent (SSMA). It uses a custom Node.js kernel (not Express) with a middleware-based architecture.

## Base URL
`http://localhost:5050` (Default)

## Authentication

SSMA now standardizes on **httpOnly session cookies** for both REST and streaming transports. There is no standalone “guest token” endpoint—anonymous clients either run without authentication (read-only SSE) or they downgrade features (e.g., CSMA’s checkout module skips the optimistic transport for guests).

### Register
- **Endpoint**: `POST /auth/register`
- **Body**: `{ "name": "Ada", "email": "ada@example.com", "password": "securepass" }`
- **Response**: `201 Created`
  ```json
  {
    "user": {
      "id": "usr_123",
      "name": "Ada",
      "email": "ada@example.com",
      "role": "user"
    }
  }
  ```
- **Side effects**: sets the `ssma_session` cookie so subsequent REST + WebSocket calls are authenticated automatically.

### Login
- **Endpoint**: `POST /auth/login`
- **Body**: `{ "email": "...", "password": "..." }`
- **Response**: `200 OK`
  ```json
  {
    "user": {
      "id": "usr_123",
      "email": "ada@example.com",
      "role": "user"
    }
  }
  ```
- **Side effects**: refreshes the session cookie.

### Logout
- **Endpoint**: `POST /auth/logout`
- **Response**: `200 OK`
- **Side effects**: clears the session cookie.

### Session probe
- **Endpoint**: `GET /auth/me`
- **Response**:
  - `200 OK` + `{ "user": { ... } }` when the cookie is valid
  - `401 Unauthorized` when no session is present

> ⚠️ **Streaming transport requirement**: the WebSocket upgrade on `/optimistic/ws` reads the exact same cookie. If `/auth/me` returns `401`, the WS handshake will also fail, which means optimistic intents will never be ACK’d. Make sure your frontend logs in (or seeds a dev session) before enabling optimistic sync for that user.

## Logging & Analytics

### 1. Ingest Log Batch
Send a batch of log entries to the backend.

- **Endpoint**: `POST /logs/batch`
- **Auth**: Required (session cookie or Authorization header — CSMA’s `LogAccumulator` still supports bearer tokens via `authProvider`)
- **Headers**:
    - `Content-Type: application/json`
- **Body**:
```json
{
  "batchId": "unique-id",
  "entries": [
    {
      "event": "CLICK",
      "message": "User failed to construct",
      "level": "info",
      "timestamp": 1700000000000
    }
  ]
}
```
- **Response**:
    - `202 Accepted`: Batch processed successfully.
    - `401 Unauthorized`: Token missing or invalid.
    - `422 Unprocessable Entity`: Schema validation failed (e.g. malformed JSON).

### 2. Health Check (Logs)
Check the status of the logging subsystem.

- **Endpoint**: `GET /logs/health`
- **Auth**: None (Public)
- **Response**: `200 OK`
```json
{
  "status": "ok",
  "exporter": {
    "type": "file",
    "exists": true
  }
}
```

## System

### 1. General Health
- **Endpoint**: `GET /health`
- **Response**: `200 OK`

## Optimistic Transport Endpoints

| Endpoint | Method | Auth | Notes |
|----------|--------|------|-------|
| `/optimistic/ws` | WebSocket | **Required** (session cookie) | Leader tabs publish `intent.batch` payloads and receive ACK/replay events. Handshake fails with `401` if `/auth/me` would fail. |
| `/optimistic/events` | GET (SSE) | Optional | Followers stream invalidations + replay snapshots. SSE stays open for guests, but it’s read-only. |
| `/optimistic/rework` | POST | Role: `staff`+ | Protected API for server-driven undo/replay. Rate limited per user/role. |
| `/admin/optimistic/*` | GET | Role: `staff`+ | Inspection endpoints (`/channels`, `/intents`) for DevOps. |

If you need anonymous users to publish optimistic actions, create a guest login route that issues a constrained cookie and reuse the same flow as real users—there is no separate “public WS” role today. Otherwise, have the frontend disable optimistic sync for those flows (see the CSMA docs for the `allowGuestCheckout` flag).
