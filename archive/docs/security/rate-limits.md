# Rate Limits

- Global HTTP rate limiter
- Rework/undo specific limiter
- Channel subscribe limiter in `SyncGateway`
- Rust gateway global WS message limiter:
  - `SSMA_RATE_WINDOW_MS` (default `60000`)
  - `SSMA_RATE_MAX` (default `120`)
- Rust gateway channel subscribe limiter:
  - `SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS` (default `10000`)
  - `SSMA_OPTIMISTIC_CHANNEL_MAX` (default `8`)
