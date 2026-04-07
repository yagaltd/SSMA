# Auth

- Session cookie based auth (`ssma_session` by default).
- Auth middleware hydrates `ctx.state.user`.
- WS auth uses cookie parsing during upgrade.
- `ssma_session` stores the signed session token used by both runtimes.
- JS and Rust gateways both verify the token during WS/SSE setup and derive the auth envelope from claims:
  - `sub` -> `user.id`
  - `role` -> `user.role` (`guest|user|staff|admin|system`)
- Missing or invalid session tokens fall back to guest access.
