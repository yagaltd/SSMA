# Backend Interface

SSMA is backend-agnostic and communicates through a narrow internal adapter contract.

## Adapter methods

### `applyIntents(batch, ctx)`
- Input:
  - `batch`: array of persisted intent entries (`id`, `intent`, `payload`, `meta`, `logSeq`, `site`)
  - `ctx`: `{ site, connectionId, ip, userAgent, user }`
  - `user`: `null` for guests, otherwise `{ id, role }`
- Output:
  - `{ results, events }`
  - `results[]`: `{ id, status, code?, message?, events? }`
  - `status` must be one of: `acked`, `rejected`, `conflict`, `failed`

### `query(name, payload, ctx)`
- Input:
  - `name`: query name
  - `payload`: query parameters
  - `ctx`: request context
- Output:
  - `{ status, data }` (canonical status `ok` for successful responses)

### `subscribe(channel, params, ctx)` (optional)
- Input:
  - `channel`: channel identifier
  - `params`: channel parameters
  - `ctx`: request context
- Output:
  - success: `{ status: 'ok', snapshot: [], cursor }`
  - unsupported: `{ status: 'error', code: 'NOT_SUPPORTED' }`

### `health(ctx)`
- Output: backend health payload, typically `{ status: 'ok' }`

## Default HTTP mapping

Default mapping used by both runtimes:
- `POST /apply-intents`
- `POST /query/:name`
- `POST /subscribe`
- `POST /health`

Runtime note:
- JS and Rust serialize the adapter context in the same camelCase JSON shape.
- Backends should treat `ctx.user` as the canonical auth envelope instead of reading transport-specific cookies.

## Failure semantics

- Transport/5xx failures map to `failed` intent status in SSMA ACKs.
- Backend semantic conflicts should return `conflict`.
- Validation/business rule rejects should return `rejected`.
