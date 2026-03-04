# Toy Backend

Minimal backend simulator for SSMA integration testing.

## Run

```bash
TOY_BACKEND_PORT=6060 node examples/toy-backend/server.mjs
```

Optional env vars:
- `SSMA_BASE_URL` (default `http://127.0.0.1:5050`)
- `SSMA_BACKEND_INTERNAL_TOKEN` (used when calling SSMA `/internal/backend/events`)

## Endpoints
- `POST /apply-intents`
- `GET /query/todos`
- `GET /metrics`
- `GET /health`
