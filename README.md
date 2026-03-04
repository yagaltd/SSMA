# SSMA (Stable State Middleware Architecture)

SSMA is a **portable backend-agnostic realtime gateway** designed for [CSMA](https://github.com/yagaltd/CSMA) frontend clients.

**Core features:**
- **Realtime sync**: WebSocket + SSE for optimistic updates and invalidations
- **CRDT support**: LWW registers, G-Counters, PN-Counters for conflict resolution
- **Backend-agnostic**: plug any backend via adapter interface
- **Pluggable storage**: JSON file (default) or SQLite

## Quick Start

Scaffold a new project with [csma-ssma-cli](https://github.com/yagaltd/csma-ssma-cli).

Or run directly from this monorepo:
```bash
npm run dev:js    # JS runtime
cargo run         # Rust runtime (from apps/ssma-rust)
```

## Repository Structure

| Path | Description |
|------|-------------|
| `apps/ssma-js` | Node.js runtime |
| `apps/ssma-rust` | Rust runtime |
| `packages/ssma-protocol` | Shared contracts & vectors |
| `docs/` | Architecture & guides |

## Templates

| Template ID | Description |
|-------------|-------------|
| `ssma-js-gateway` | JS runtime |
| `ssma-rust-gateway` | Rust runtime |

## Development

```bash
npm run dev:js              # JS dev server
npm run test:js             # JS tests
npm run test:conformance    # Protocol conformance
npm run test:rust           # Rust tests
npm run validate:templates  # Validate template manifests
```

For architecture details, see [docs/](docs/).

## Acknowledgements

* Inspired by [Logux](https://github.com/logux)
