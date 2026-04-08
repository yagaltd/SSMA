---
name: ssma-protocol
description: Work on wire protocol, message contracts, schema validation, or subprotocol versioning. Use when changing protocol.rs, contracts, or vectors.
---

# SSMA Protocol

## Contract Sources

Canonical schemas: `packages/ssma-protocol/contracts/`

- `optimistic.json` — INTENT_BATCH, PING
- `channels.json` — CHANNEL_SUBSCRIBE, CHANNEL_UNSUBSCRIBE, CHANNEL_RESYNC, CHANNEL_COMMAND
- `errors.json` — ERROR_FRAME

Golden vectors: `packages/ssma-protocol/vectors/`

- `ws_handshake.json`
- `intent_batch_ack.json`
- `replay_window.json`
- `channel_subscribe_snapshot.json`
- `rate_limit_channel_subscribe.json`
- `unauthorized_ws_reject.json`

## Subprotocol Versioning

- Config: `SSMA_PROTOCOL_SUBPROTOCOL` (default `1.0.0`)
- Legacy alias: `SSMA_OPTIMISTIC_SUBPROTOCOL` (deprecated)
- Rule: major version must match between client and server
- Mismatch: WS `error` frame with code `SUBPROTOCOL_MISMATCH`, then close

## WS Client → Server Messages

| Message | Contract | Required fields | Notes |
|---------|----------|-----------------|-------|
| `intent.batch` | `optimistic.INTENT_BATCH` | `type`, `intents[]` | Write path; leader-only |
| `channel.subscribe` | `channels.CHANNEL_SUBSCRIBE` | `type`, `channel` | `params` optional but preserved downstream |
| `channel.unsubscribe` | `channels.CHANNEL_UNSUBSCRIBE` | `type`, `channel` | `params` selects exact subscription |
| `channel.resync` | `channels.CHANNEL_RESYNC` | `type`, `channel` | `params` keeps resync scoped |
| `channel.command` | `channels.CHANNEL_COMMAND` | `type`, `channel`, `command` | `params` forwarded with response |
| `ping` | `optimistic.PING` | `type` | Returns `pong` |

## WS Server → Client Messages

| Message | Key fields | Notes |
|---------|------------|-------|
| `hello` | `subprotocol`, `connectionId` | First frame after handshake |
| `ack` | `intents[]` (`id`, `status`, `logSeq`) | Per-intent result map |
| `replay` | `intents[]`, `cursor` | Sent on connect; may be empty |
| `channel.ack` | `status`, `channel`, `params?` | Subscribe acknowledgement |
| `channel.snapshot` | `channel`, `params`, `intents[]`, `cursor` | Initial channel state |
| `channel.replay` | `status`, `channel`, `params`, `intents[]`, `cursor` | Resync payload |
| `channel.invalidate` | `channel`, `params`, `intents[]`, `cursor` | Single-channel fanout |
| `channel.close` | `channel`, `code` | Subscription closure |
| `channel.command` | `status`, `command`, `params?` | Command response |
| `error` | `code`, `message?` | Structured error frame |

## SSE Events

| Event | Payload |
|-------|---------|
| `ready` | Client bootstrap metadata |
| `replay` | Initial intent replay + cursor |
| `invalidate` | Intent invalidation batch |
| `island.invalidate` | Island-level invalidation |
| `server.shutdown` | Graceful shutdown signal |

## Error Codes

- `INVALID_JSON`
- `INVALID_CONTRACT`
- `SUBPROTOCOL_MISMATCH`
- `UNAUTHORIZED`
- `RATE_LIMITED`
- `PAYLOAD_TOO_LARGE`
- `UNKNOWN_TYPE`

## Durable vs Ephemeral Rule

- **Durable**: persisted user/app actions via `intent.batch` → replayable
- **Ephemeral**: RTC signaling, audio events, model token deltas → fan out as `channel.invalidate` but do NOT enter replay store

## Schema Validation

`protocol.rs` loads contracts at startup via `Lazy<HashMap<String, JSONSchema>>`.

`validate_inbound(payload)` checks the message `type`, finds the matching contract, and validates against the compiled JSON schema.

## Modifying Contracts

1. Edit the relevant `.json` in `packages/ssma-protocol/contracts/`
2. Add or update a golden vector in `packages/ssma-protocol/vectors/`
3. Run `cargo test` — the conformance and E2E tests exercise the vectors
4. Update this SKILL.md if message types change
