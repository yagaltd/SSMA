# SSMA Tauri Integration

**Status**: Proposal  
**Date**: 2026-03-10

## Summary

Tauri support is a good ecosystem direction for SSMA, but it should start as a thin integration path, not as a new runtime.

The recommended path is:
- embed the existing Rust SSMA gateway inside the Tauri app
- keep the current HTTP/WebSocket contract
- let CSMA reuse its existing optimistic transport

Do not start with:
- a Tauri-only SSMA runtime
- a Tauri-specific wire protocol
- a new CSMA transport module

## Why This Makes Sense

SSMA already has a Rust runtime.
Tauri already runs a Rust backend.
That means the shortest path is to reuse `apps/ssma-rust` behavior and wrap it in a Tauri integration template.

Value:
- fast adoption for desktop apps
- no protocol fork
- less maintenance than building a third SSMA runtime
- CSMA compatibility stays straightforward

## Recommended Architecture

### Option 1: Embedded Gateway

Run the existing Rust SSMA gateway inside the Tauri backend and expose it on a local endpoint used by the frontend.

Shape:
- Tauri app starts
- embedded SSMA Rust gateway starts
- CSMA frontend connects to local WS/SSE endpoints
- Tauri shutdown closes the gateway cleanly

Why this is the default:
- preserves current contracts
- reuses tested gateway behavior
- avoids a second transport model

### Option 2: Reusable Tauri Plugin

If multiple apps repeat the same embedded setup, package it as a reusable Tauri plugin.

This should still preserve:
- the same backend adapter contract
- the same optimistic wire protocol
- the same auth and RBAC semantics

This is a later step, not the first step.

### Option 3: Tauri-Native IPC Runtime

This means replacing HTTP/WebSocket with Tauri command/event IPC.

Do not build this now.

Only consider it if:
- Tauri becomes a strategic target
- embedded HTTP/WebSocket becomes measurably inadequate
- the project deliberately commits to a Tauri IPC contract

## CSMA Impact

Default position:
- CSMA should not get a new Tauri transport module yet

Reason:
- if SSMA is embedded and still exposes local HTTP/WebSocket, CSMA can keep using its existing transport
- the real need is a Tauri template or setup path, not a transport rewrite

Only build a CSMA Tauri transport if:
- the architecture moves to pure Tauri IPC
- the team accepts the maintenance cost of a platform-specific transport

## Scope To Build Now

### Phase 1

Build:
- a Tauri example or template using embedded `ssma-rust`
- startup and shutdown wiring
- frontend configuration guidance for local endpoint discovery
- desktop-first documentation

Acceptance criteria:
- app boots with embedded SSMA
- CSMA can connect with current optimistic transport
- shutdown does not leak sockets, listeners, or reconnect loops

### Phase 2

Build:
- multi-window lifecycle guidance
- auth/session guidance for embedded Tauri apps
- packaging notes for desktop targets

Acceptance criteria:
- one documented desktop setup works end-to-end
- auth and invalidation behavior matches the current SSMA contract

### Phase 3

Evaluate:
- whether a reusable `tauri-plugin-ssma` is justified

Build it only if:
- multiple apps repeat the same integration work
- the plugin clearly reduces duplication without introducing a new contract

## Do Not Build Now

- a new `apps/ssma-tauri` runtime
- a Tauri-specific protocol
- a Tauri-only CSMA optimistic transport

## Relationship To SSMA Core

Tauri support should stay outside SSMA core at first.
Treat it as:
- template work
- integration work
- optional ecosystem support

SSMA core should continue to focus on:
- JS/Rust runtime parity
- protocol correctness
- lifecycle robustness
- backend adapter contract stability

## Related Files

- `roadmap.md`
- `docs/guides/SSMA-RUNTIME.md`
- `docs/protocol/wire-protocol.md`
- `docs/architecture/backend-interface.md`
