# SSMA Future Enhancements

This roadmap captures the remaining backend work that can follow the completed optimistic-sync + auth foundation. Each item is phrased as an outcome so we can evaluate impact before implementation.

## 1. Authentication & Authorization Extras

| Workstream | Goal | Notes |
|------------|------|-------|
| Password reset & verification | Let operators issue reset links/tokens and enforce verified email state before granting elevated roles. | Reuse `AuthService` with short-lived signed tokens + rate-limited endpoints. |
| OAuth / SSO integration | Support enterprise identity (Google, Okta, Azure AD) using [`oauth4webapi`](https://github.com/panva/oauth4webapi`). | Keep CSMA cookies/JWTs as session carriers; map IdP claims → SSMA roles. |
| Admin user management APIs | CRUD interfaces for staff/system operators to manage users and roles, with audit logs. | Build on existing RBAC utilities. |

## 2. DevTools & Admin Experience

| Workstream | Goal | Notes |
|------------|------|-------|
| Admin dashboard | Surface `/admin/optimistic/*` data, LogService streams, and rate-limit alerts in a web UI. | Could live inside SSMA or as a separate ops console backed by the new APIs. |
| Transport telemetry | Emit OpenTelemetry/metrics (latency, replay depth, rate-limit counts) for dashboards/alerts. | Wrap `ingestServerEvent` exporter or add OTLP hooks. |
| CLI tooling | Provide scripts to query pending intents, force rework/undo, or toggle channels without using raw HTTP calls. | Useful for on-call workflows. |

## 3. Storage & Durability

| Adapter | Goal | Notes |
|--------|------|-------|
| RocksDB | High-write, low-latency persistence suitable for local-first desktop apps. | Reuse the adapter contract; ship with compaction + backup hooks. |
| Postgres (or other SQL) | Multi-node deployments with replication, migrations, and SQL analytics. | Map `meta` JSON into JSONB columns, keep reason indexes. |
| Object storage mirroring | Optional archival of intent batches (S3, GCS) for compliance. | Trigger async writers from `IntentStore.append`. |

### CSMA client log adapters

| Adapter | Goal | Notes |
|---------|------|-------|
| IndexedDB (existing) | Default browser persistence for optimistic actions. | Already shipped in `ActionLogService` with BroadcastChannel sync. |
| File System Access API | Persist action logs to the user’s disk (desktop-class PWAs) for >10k entries and offline recovery. | Not implemented yet—requires new `FileSystemActionStore` plugged into the CSMA module plus permission UX. |
| Hybrid cloud backup | Optional upload of client action logs to SSMA for diagnostics. | Future stretch; depends on File System adapter groundwork. |

## 4. Transport & Channel Policies

- **Replay tuning**: expose per-channel replay windows, e.g., hot channels keep 24h history while others use the default 5 minutes.
- **Channel-level quotas**: allow ops to define max subscribers / rate per channel, with auto-block + telemetry when exceeded.
- **Multi-site federation**: support multiple CSMA sites syncing to a shared SSMA cluster, with site-based isolation and cross-site invalidations.
- **Cloudflare Workers transport**: build a lightweight gateway variant (Durable Objects + WebSocket/SSE polyfills) so SSMA transport can run on Workers for edge deployments; requires adapting storage + auth layers to worker-compatible APIs.
- **Serverless/edge deployment kit**: extend the new `deploy:cdn` / `deploy:vps` flow with a third track that packages CSMA assets for Workers/Pages and swaps SSMA’s Node kernel for a workers-compatible adapter (KV/D1 storage, Durable Objects transport, and per-request auth). Includes infra scripts, env scaffolding, and documentation for Cloudflare + Netlify Edge + Vercel Edge runtimes.

## 5. Operational Hardening

- **Disaster recovery playbooks**: document backup/restore for SQLite, RocksDB, or Postgres adapters plus sample scripts.
- **Structured audit log shipping**: forward `LogService` output to ELK/Datadog/Splunk with schema guarantees.
- **Performance benchmarking**: include load-test scenarios (intent throughput, concurrent WS connections) to guard regressions.

> ✅ Completed baseline: optimistic transport (WS + SSE), IndexedDB/SQLite parity, auth + RBAC, channel registry, rework/undo APIs, rate limiting, and DevTools integrations. The items above build on that base without blocking current adopters.
