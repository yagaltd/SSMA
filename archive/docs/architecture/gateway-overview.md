# Gateway Overview

SSMA is the sync authority. Browser clients connect only to SSMA.

Responsibilities:
- auth + RBAC + rate limit
- contract validation
- WS/SSE transport
- intent persistence + replay
- forwarding to backend adapter
- invalidation fanout
