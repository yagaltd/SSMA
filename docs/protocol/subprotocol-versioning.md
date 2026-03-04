# Subprotocol Versioning

- Server setting: `SSMA_PROTOCOL_SUBPROTOCOL`
- Legacy alias: `SSMA_OPTIMISTIC_SUBPROTOCOL` (deprecated fallback)
- Compatibility rule: major version must match.
- Mismatch result: WS `error` with code `SUBPROTOCOL_MISMATCH` and close.
