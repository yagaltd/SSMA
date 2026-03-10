# RBAC

Role checks enforce protected optimistic channels using role ranking:

Examples:
- `ops.audit` channel requires staff role.
- write auth enforcement can be enabled for leaders.
- Rust gateway protected channels are configured with:
  - `SSMA_OPTIMISTIC_PROTECTED_CHANNELS` (comma-separated names)
  - `SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE` (default: `admin`)
- Both runtimes resolve the caller role from the verified `ssma_session` token claims and deny protected-channel access when below threshold (`CHANNEL_ACCESS_DENIED` event + `channel.close`).
- Island invalidations use the same role ranking before fanout on WS and SSE.
