# Deployment Notes

- Keep backend private; expose SSMA as edge-facing gateway.
- Ensure reverse proxy supports WS and SSE upgrades/streaming.
- For horizontal scale, back intent store with sqlite/external durable storage.
