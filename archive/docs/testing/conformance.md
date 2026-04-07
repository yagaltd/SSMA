# Conformance

Protocol vectors are maintained under `packages/ssma-protocol/vectors/`.

## Run conformance checks

- `npm --prefix apps/ssma-js run test:conformance`

This runs `apps/ssma-js/tests/conformance-vectors.test.js`, which replays vector scenarios against the JS gateway handlers and asserts emitted protocol frame shapes.

## Current vectors

- `ws_handshake.json`
- `intent_batch_ack.json`
- `replay_window.json`
- `channel_subscribe_snapshot.json`
- `unauthorized_ws_reject.json`
- `rate_limit_channel_subscribe.json`

## Normalization rules

To keep vectors deterministic across runtimes:
- compare stable fields (`type`, `status`, `code`, `id`)
- allow dynamic values (`connectionId`, timestamps, generated cursors/logSeq) to be asserted by shape/range

## Cross-runtime parity

Rust runtime should implement the same vector replay assertions and pass identical message-shape expectations.
