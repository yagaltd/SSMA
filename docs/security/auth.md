# Auth

- Session cookie based auth (`ssma_session` by default).
- Auth middleware hydrates `ctx.state.user`.
- WS auth uses cookie parsing during upgrade.
- Rust gateway WS extraction currently uses cookies:
  - `ssma_session` -> `user_id`
  - `ssma_role` -> RBAC role (`guest|user|staff|admin|system`)
