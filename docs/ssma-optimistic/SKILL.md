---
name: ssma-optimistic
description: Work on intent persistence, replay, channel subscriptions, or fanout logic. Use when modifying domain/runtime.rs, features/optimistic.rs, or channel behavior.
---

# SSMA Optimistic Sync

## Core Concept

SSMA persists optimistic intents from clients and replays them to reconnecting clients. This enables offline-first and multi-tab sync.

## Intent Store (`domain/runtime.rs`)

### IntentRecord

```rust
pub struct IntentRecord {
    pub id: String,           // Client-generated unique ID
    pub intent: String,       // Action type (e.g., "TODO_CREATE")
    pub payload: Value,       // Action data
    pub meta: Value,          // { clock, channels, reasons }
    pub inserted_at: i64,     // Milliseconds since epoch
    pub log_seq: u64,         // Monotonic cursor
    pub site: String,         // Site identifier
    pub status: String,       // acked | pending | rejected | conflict | failed
    pub connection_id: Option<String>,
    pub actor_key: Option<String>,
    pub user_id: Option<String>,
    pub backend: Option<Value>,
}
```

### Deduplication

Key: `{site}::{id}` — same intent ID on same site is deduplicated.

When a duplicate arrives:
- Original record is returned in `replayed`
- No new `log_seq` is assigned

### Persistence

- JSON file at `SSMA_OPTIMISTIC_STORE` (default `./data/optimistic-intents-rust.json`)
- Atomic writes: write to `.json.tmp`, then `rename()`
- Loaded on startup, rebuilt into in-memory index

### Replay Window

- Config: `SSMA_OPTIMISTIC_REPLAY_MS` (default 5 minutes)
- Entries older than `now - replay_window` are trimmed
- Max entries: `SSMA_OPTIMISTIC_MAX_ENTRIES` (default 5000)

## Intent Batch Flow

1. WS leader sends `intent.batch` with `intents[]`
2. Schema validation against `optimistic.INTENT_BATCH`
3. Auth check (if `SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES`)
4. Rate limit check (global + per-actor)
5. `IntentStore.append_batch()` — dedupe, assign `log_seq`, persist
6. Forward fresh intents to backend via `POST /apply-intents`
7. Normalize backend statuses
8. Send `ack` to the leader
9. Broadcast `invalidate` to all connections
10. Send `channel.invalidate` to channel subscribers

## Channel Subscriptions

### Subscribe Flow

1. Client sends `{ type: "channel.subscribe", channel: "todos", params: {} }`
2. Rate limit check (per-connection burst limit)
3. RBAC check (protected channels)
4. Backend query via `POST /subscribe`
5. Send `channel.ack` (status: ok)
6. Send `channel.snapshot` (initial state + cursor)

### Invalidation Fanout

When intents are persisted with `meta.channels: ["todos"]`:
1. Find all connections subscribed to `todos`
2. For each subscriber: send `channel.invalidate` with the intents
3. `channel.invalidate` targets ONE `channel` at a time (not `channels[]`)

### Unsubscribe

Client sends `{ type: "channel.unsubscribe", channel: "todos", params: {} }`
→ Removes the subscription from the registry

### Resync

Client sends `{ type: "channel.resync", channel: "todos", params: {} }`
→ Returns `channel.replay` with intents since the subscription's cursor

## Rework & Undo (`features/optimistic.rs`)

### Rework (staff+)

`POST /optimistic/rework` with `{ id, site, reason? }`
- Re-adds `pending` reason to the intent
- Broadcasts invalidation
- Rate-limited by `SSMA_OPTIMISTIC_REWORK_*`

### Undo (authenticated)

`POST /optimistic/undo` with `{ id, site, intent, payload, reason? }`
- Validates ownership (user owns the intent)
- Stores undo metadata
- Broadcasts invalidation

### Pending Query

`GET /optimistic/pending?since=<cursor>&limit=<n>&site=<site>`
- Returns intents with active `pending` reason
- Requires authentication

## Server Events

Key events emitted during optimistic flow:

- `INTENT_ACKED` — batch accepted
- `INTENT_REJECTED` — batch rejected (auth, validation, rate limit)
- `CHANNEL_SUBSCRIBE` — new subscription
- `CHANNEL_UNSUBSCRIBE` — subscription removed
- `CHANNEL_ACCESS_DENIED` — RBAC denied
- `CHANNEL_SNAPSHOT_SENT` — snapshot delivered
- `OPTIMISTIC_REWORK_RATE_LIMIT` — rework rate limited

## Durable vs Ephemeral

| Type | Durable | Replayable |
|------|---------|------------|
| User intents (`intent.batch`) | ✅ | ✅ |
| Channel snapshots | ❌ | N/A |
| RTC signaling | ❌ | ❌ |
| Audio events | ❌ | ❌ |
| Backend invalidations | ❌ | ❌ |

## Key Files

- `domain/runtime.rs` — IntentStore, append_batch, deduplication, persistence
- `features/optimistic.rs` — Rework, undo, pending queries
- `transport/mod.rs` — Channel registry, subscription management, fanout helpers
- `transport/ws.rs` — handle_intent_batch, channel subscribe/unsubscribe/resync
