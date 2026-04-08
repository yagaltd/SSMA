---
name: ssma-backend
description: Integrate a backend adapter with SSMA. Use when implementing applyIntents, query, subscribe, or handling media/RTC from the backend side.
---

# SSMA Backend Adapter

SSMA is backend-agnostic. Your backend implements a narrow HTTP adapter contract.

## Adapter Endpoints

SSMA calls your backend at these routes:

| Route | Method | Purpose |
|-------|--------|---------|
| `/apply-intents` | POST | Process persisted intents |
| `/query/:name` | POST | Handle one-shot queries |
| `/subscribe` | POST | Initialize channel subscription |
| `/health` | POST | Backend health check |

Base URL: `SSMA_BACKEND_URL`

## `POST /apply-intents`

### Request

```json
{
  "intents": [
    {
      "id": "client-uuid-1",
      "intent": "TODO_CREATE",
      "payload": { "id": "todo-1", "title": "Buy milk" },
      "meta": { "clock": 1, "channels": ["todos"] },
      "logSeq": 42,
      "site": "default"
    }
  ],
  "context": {
    "site": "default",
    "actorKey": "user:user-uuid",
    "connectionId": "conn-uuid",
    "ip": "192.168.1.1",
    "userAgent": "Mozilla/5.0",
    "user": { "id": "user-uuid", "role": "user" }
  }
}
```

### Response

```json
{
  "results": [
    {
      "id": "client-uuid-1",
      "status": "acked"
    }
  ]
}
```

Status values: `acked`, `rejected`, `conflict`, `failed`

## `POST /query/:name` (JSON response)

### Request

```json
{
  "payload": { "productId": "abc-123" },
  "context": {
    "site": "default",
    "actorKey": "user:user-uuid",
    "user": { "id": "user-uuid", "role": "user" }
  }
}
```

### Response

```json
{
  "status": "ok",
  "data": { "product": { "name": "Widget", "price": 9.99 } }
}
```

## `POST /query/:name` (streaming, NDJSON)

When SSMA calls with `Accept: application/x-ndjson` header and `stream: true` in body:

### Request

```json
{
  "payload": { "prompt": "Hello" },
  "context": { ... },
  "stream": true
}
```

### Response (NDJSON)

Each line is a JSON object, flushed as available:

```json
{"type":"chunk","delta":"Hello"}
{"type":"chunk","delta":" world"}
{"type":"done"}
```

SSMA forwards these as SSE events to the client at `/query/:name/stream`.

---

## `POST /subscribe`

### Request

```json
{
  "channel": "todos",
  "params": { "listId": "inbox" },
  "context": {
    "site": "default",
    "actorKey": "user:user-uuid",
    "user": { "id": "user-uuid", "role": "user" }
  }
}
```

### Response (success)

```json
{
  "status": "ok",
  "snapshot": [
    { "id": "todo-1", "title": "Buy milk", "done": false }
  ],
  "cursor": 100
}
```

### Response (unsupported)

```json
{
  "status": "error",
  "code": "NOT_SUPPORTED"
}
```

## `POST /health`

### Request

```json
{
  "context": {
    "site": "default",
    "actorKey": null,
    "user": null
  }
}
```

### Response

```json
{
  "status": "ok"
}
```

## Context Object

Every adapter call includes `context`:

```typescript
interface BackendContext {
  site: string;
  actorKey: string | null;      // Stable ownership key
  connectionId: string | null;
  ip: string | null;
  userAgent: string | null;
  user: {
    id: string;
    role: "guest" | "user" | "staff" | "admin" | "system";
  } | null;  // null for guests
}
```

## Media Handling

Frontends upload media to SSMA, not to your backend.

### Backend Reading Media

Use internal routes with `x-ssma-backend-token`:

```
GET /internal/assets/:assetId
GET /internal/assets/:assetId/content
```

### Backend Creating Media

Upload to SSMA internal route:

```
POST /internal/assets
```

Include in request:
- `multipart/form-data` with file
- `site`, `actorKey`, `mediaType`, `mimeType` fields

## Backend Events

Push invalidations to SSMA:

```
POST /internal/backend/events
```

Headers: `x-ssma-backend-token: <token>`

```json
{
  "events": [
    {
      "site": "default",
      "reason": "external-update",
      "islandId": "product-inventory",
      "intents": [{ "id": "ext-1", "intent": "STOCK_UPDATE", "payload": {} }]
    }
  ]
}
```

## Experimental Audio / WebRTC

Audio and WebRTC files may exist under `src/experimental/`, but they are not part of the active SSMA gateway surface.

Do not add backend contracts for audio/WebRTC unless that feature is explicitly brought back into the runtime and documented in transport docs.

## Failure Semantics

| Scenario | Return |
|----------|--------|
| Success | `status: "acked"` |
| Business rule violation | `status: "rejected"` |
| Conflict (e.g., concurrent edit) | `status: "conflict"` |
| Transport/server error | `status: "failed"` |

## Unconfigured Backend

When `SSMA_BACKEND_URL` is empty:
- `applyIntents` → returns `{ results: [] }` (silent no-op)
- `query` → returns `{ status: "ok", data: null }`
- `subscribe` → returns `{ status: "ok", snapshot: [], cursor: 0 }`
- `health` → returns `{ status: "ok", backend: "unconfigured" }`
