# Backend Interface

SSMA is backend-agnostic and communicates through a narrow internal adapter contract.

## Adapter methods

### `applyIntents(batch, ctx)`
- Input:
  - `batch`: array of persisted intent entries (`id`, `intent`, `payload`, `meta`, `logSeq`, `site`)
  - `ctx`: `{ site, actorKey, connectionId, ip, userAgent, user }`
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
- `ctx.actorKey` is the stable ownership key for anonymous or authenticated callers and is required when backends create SSMA-owned output assets.

## Media-bearing workloads

- Frontends upload binary image/audio files to SSMA, not to adapters.
- SSMA returns gateway-owned `assetId` handles.
- Intents/queries that reference media should carry `assetRefs[]`, not raw or base64 payloads.
- Backend adapters resolve asset refs through SSMA internal routes:
  - `POST /internal/assets`
  - `GET /internal/assets/:assetId`
  - `GET /internal/assets/:assetId/content`
  - `DELETE /internal/assets/:assetId`
- Internal asset routes are protected by `x-ssma-backend-token`.

Session A note:
- Rust runtime implements the SSMA-owned media boundary and internal asset fetch contract.
- JS runtime parity is still pending.

## Realtime audio session contract (Rust-first)

Session B adds an SSMA-owned realtime audio session layer on top of RTC signaling.

Public gateway responsibilities:
- `POST /audio/sessions`
- `GET /audio/sessions/:sessionId`
- `DELETE /audio/sessions/:sessionId`
- `POST /audio/sessions/:sessionId/commands`
- RTC signaling remains on `POST /rtc/sessions` and `POST /rtc/sessions/:sessionId/signals`

Adapter expectations:
- `applyIntents` should accept realtime audio control intents such as:
  - `models.local.audio.session.start`
  - `models.local.audio.session.stop`
  - `models.local.audio.session.command`
- `subscribe(channel, params, ctx)` should support snapshots for `audio.session.<audioSessionId>` when the backend owns live session state.
- backend fanout events may be sent to SSMA `/internal/backend/events` with:
  - `site`
  - `audioSessionId`
  - `eventType`
  - `payload`

Expected ephemeral event types:
- `audio.session.started`
- `audio.session.partial_transcript`
- `audio.session.final_transcript`
- `audio.session.audio_out_started`
- `audio.session.audio_out_chunk`
- `audio.session.audio_out_stopped`
- `audio.session.interrupted`
- `audio.session.ended`
- `audio.session.error`

Important rule:
- realtime audio events are ephemeral stream events and must **not** be treated as durable optimistic replay state.

## Failure semantics

- Transport/5xx failures map to `failed` intent status in SSMA ACKs.
- Backend semantic conflicts should return `conflict`.
- Validation/business rule rejects should return `rejected`.
