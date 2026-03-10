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

## Error codes

Common codes:
- `INVALID_JSON`
- `INVALID_CONTRACT`
- `SUBPROTOCOL_MISMATCH`
- `UNAUTHORIZED`
- `RATE_LIMITED`
- `UNKNOWN_TYPE`
