# Wire Protocol

Canonical schemas live in `packages/ssma-protocol/contracts/`.

## WS client -> server

| Message | Contract | Required fields | Notes |
|---|---|---|---|
| `intent.batch` | `optimistic.INTENT_BATCH` | `type`, `intents[]` | Write path; leader-only. |
| `channel.subscribe` | `channels.CHANNEL_SUBSCRIBE` | `type`, `channel` | `params` is optional on the wire but preserved in all downstream frames for that subscription. |
| `channel.unsubscribe` | `channels.CHANNEL_UNSUBSCRIBE` | `type`, `channel` | `params` selects the exact subscription instance to close. |
| `channel.resync` | `channels.CHANNEL_RESYNC` | `type`, `channel` | `params` keeps resync scoped to the original subscription. |
| `channel.command` | `channels.CHANNEL_COMMAND` | `type`, `channel`, `command` | `params` is forwarded with the command response. |
| `ping` | `optimistic.PING` | `type` | Returns `pong`. |

## WS server -> client

| Message | Key fields | Notes |
|---|---|---|
| `hello` | `subprotocol`, `connectionId` | First frame after handshake success. |
| `ack` | `intents[]` (`id`,`status`,`logSeq`) | Per-intent result map. |
| `replay` | `intents[]`, `cursor` | Sent on connect; may be empty. |
| `channel.ack` | `status`, `channel`, `params?` | Subscribe acknowledgement / errors. |
| `channel.snapshot` | `channel`, `params`, `intents[]`, `cursor` | Initial channel state for one scoped subscription. |
| `channel.replay` | `status`, `channel`, `params`, `intents[]`, `cursor` | Resync payload for one scoped subscription. |
| `channel.invalidate` | `channel`, `params`, `intents[]`, `cursor` | Channel invalidation fanout. Uses a single `channel`, not `channels[]`. |
| `channel.close` | `channel`, `code` | Subscription closure reason. |
| `channel.command` | `status`, `command`, `params?` | Command response. |
| `error` | `code`, `message?` | Contract in `errors.ERROR_FRAME`. |

## SSE server -> client

| Event | Payload summary |
|---|---|
| `ready` | client bootstrap metadata |
| `replay` | initial intent replay + cursor |
| `invalidate` | intent invalidation batch |
| `island.invalidate` | island-level invalidation payload |
| `rework` / `undo` | optional operational events |

Session A rule:
- model token deltas, RTC signaling, and other ephemeral stream events may fan out as `invalidate` payloads on request/session channels, but they do **not** enter the durable replay store.
- replay is for persisted optimistic intents only.

## HTTP media + RTC (Rust Session A)

| Route | Purpose |
|---|---|
| `POST /query/:name` | Public request/response query path that forwards to the configured backend adapter. |
| `POST /media/assets` | Upload binary image/audio and receive an `assetId`. |
| `GET /media/assets/:assetId` | Asset metadata for the current owner/session. |
| `GET /media/assets/:assetId/content` | Raw asset bytes for the current owner/session. |
| `DELETE /media/assets/:assetId` | Release uploaded asset. |
| `POST /rtc/sessions` | Create a signaling session and return `rtc.session.<id>` channel name. |
| `POST /rtc/sessions/:sessionId/signals` | Submit an offer/answer/candidate-style signal payload. |

Internal adapter fetch:
- `POST /internal/assets`
- `GET /internal/assets/:assetId`
- `GET /internal/assets/:assetId/content`
- `DELETE /internal/assets/:assetId`
- Both require `x-ssma-backend-token`.

Session A follow-up rule:
- public queries forward `ctx.actorKey` so adapters can create backend-generated assets owned by the same caller
- backend-created assets use `/internal/assets` and become readable through normal `/media/assets/:assetId/content`

## Error codes

Common codes:
- `INVALID_JSON`
- `INVALID_CONTRACT`
- `SUBPROTOCOL_MISMATCH`
- `UNAUTHORIZED`
- `UNAUTHORIZED_BACKEND_REQUEST`
- `RATE_LIMITED`
- `PAYLOAD_TOO_LARGE`
- `UNKNOWN_TYPE`
