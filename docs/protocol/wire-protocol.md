# Wire Protocol

Canonical schemas live in `packages/ssma-protocol/contracts/`.

## WS client -> server

| Message | Contract | Required fields | Notes |
|---|---|---|---|
| `intent.batch` | `optimistic.INTENT_BATCH` | `type`, `intents[]` | Write path; leader-only. |
| `channel.subscribe` | `channels.CHANNEL_SUBSCRIBE` | `type`, `channel` | Returns `channel.ack` + optional `channel.snapshot`. |
| `channel.unsubscribe` | `channels.CHANNEL_UNSUBSCRIBE` | `type`, `channel` | Returns `channel.unsubscribed`. |
| `channel.resync` | `channels.CHANNEL_RESYNC` | `type`, `channel` | Returns `channel.replay`. |
| `channel.command` | `channels.CHANNEL_COMMAND` | `type`, `channel`, `command` | Returns `channel.command`. |
| `ping` | `optimistic.PING` | `type` | Returns `pong`. |

## WS server -> client

| Message | Key fields | Notes |
|---|---|---|
| `hello` | `subprotocol`, `connectionId` | First frame after handshake success. |
| `ack` | `intents[]` (`id`,`status`,`logSeq`) | Per-intent result map. |
| `replay` | `intents[]`, `cursor` | Sent on connect; may be empty. |
| `channel.ack` | `status`, `channel` | Subscribe acknowledgement / errors. |
| `channel.snapshot` | `channel`, `intents[]`, `cursor` | Initial channel state. |
| `channel.replay` | `status`, `intents[]`, `cursor` | Resync payload. |
| `channel.invalidate` | `channel`, `intents[]`, `cursor` | Channel invalidation fanout. |
| `channel.close` | `channel`, `code` | Subscription closure reason. |
| `channel.command` | `status`, `command` | Command response. |
| `error` | `code`, `message?` | Contract in `errors.ERROR_FRAME`. |

## SSE server -> client

| Event | Payload summary |
|---|---|
| `ready` | client bootstrap metadata |
| `replay` | initial intent replay + cursor |
| `invalidate` | intent invalidation batch |
| `island.invalidate` | island-level invalidation payload |
| `rework` / `undo` | optional operational events |

## Error codes

Common codes:
- `INVALID_JSON`
- `INVALID_CONTRACT`
- `SUBPROTOCOL_MISMATCH`
- `UNAUTHORIZED`
- `RATE_LIMITED`
- `UNKNOWN_TYPE`
