---
name: ssma-testing
description: Write or run tests for the SSMA gateway. Use when adding E2E tests, unit tests, or conformance checks.
---

# SSMA Testing

## Commands

```bash
# All tests
cd apps/ssma-rust && cargo test -- --nocapture

# Specific test file
cd apps/ssma-rust && cargo test --test e2e_auth -- --nocapture

# Specific test
cd apps/ssma-rust && cargo test register_returns_201 -- --nocapture

# Unit tests only (lib)
cd apps/ssma-rust && cargo test --lib -- --nocapture

```

## Test Structure

```
apps/ssma-rust/
├── src/
│   ├── transport/
│   │   ├── auth.rs      # mod tests {} — password hashing, JWT
│   │   └── mod.rs       # mod tests {} — role_rank, normalize_status
│   └── features/
│       └── media.rs     # mod tests {} — MIME type detection
├── tests/
│   ├── e2e_auth.rs          # Register, login, logout, /me
│   ├── e2e_admin.rs         # Admin endpoints (staff+)
│   ├── e2e_cors.rs          # CORS headers
│   ├── e2e_health.rs        # Health endpoint
│   ├── e2e_logs.rs          # Log relay forwarding
│   ├── e2e_forms.rs         # Form ingress, honeypot, captcha hooks
│   ├── e2e_webhooks.rs      # Webhook verify/idempotency/forwarding
│   ├── e2e_oidc.rs          # OIDC start/callback bridge
│   ├── e2e_optimistic_ops.rs # Rework, undo, pending
│   ├── e2e_scenarios.rs     # Full integration scenarios
│   ├── e2e_ws.rs            # WebSocket handler tests
│   ├── conformance_runtime.rs # Protocol vector replay
│   ├── store_and_backend.rs  # IntentStore + backend adapter
│   └── vector_harness.rs     # Vector test harness
```

## Writing E2E Tests

### Pattern: In-Process (fast)

Use `tower::ServiceExt::oneshot` for HTTP tests without binding a port:

```rust
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;
use ssma_rust::config::Config;
use ssma_rust::gateway;

fn test_config(tmp: &std::path::Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config
}

#[tokio::test]
async fn my_test() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```

### Pattern: Real Server (for WS/SSE)

Bind to port 0 for OS-assigned port:

```rust
use tokio::net::TcpListener;

async fn spawn_server(config: Config) -> (String, tokio::task::JoinHandle<()>) {
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (format!("127.0.0.1:{}", addr.port()), handle)
}
```

### Pattern: WebSocket Test

```rust
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ws_test() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    // Connect as leader to send intents
    let url = format!("ws://{}/optimistic/ws?role=leader", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    // Wait for hello
    // ... receive and assert frames ...

    handle.abort();
}
```

### Pattern: Authenticated Requests

```rust
fn make_cookie(config: &Config, user_id: &str, role: &str) -> String {
    use jsonwebtoken::{encode, EncodingKey, Header};
    let now = ssma_rust::runtime::now_secs();
    let claims = serde_json::json!({
        "sub": user_id,
        "role": role,
        "iss": config.jwt_issuer,
        "aud": config.jwt_audience,
        "iat": now,
        "exp": now + 3600,
    });
    let jwt = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.auth_jwt_secret.as_bytes()),
    ).unwrap();
    format!("ssma_session={}", jwt)
}

// Usage
let cookie = make_cookie(&config, "user-1", "staff");
let req = Request::builder()
    .header("Cookie", &cookie)
    // ...
```

## Writing Unit Tests

Add `#[cfg(test)] mod tests {}` at the bottom of the source file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_unit_test() {
        assert_eq!(role_rank("admin"), 3);
    }
}
```

## Golden Vector Conformance

Vectors in `packages/ssma-protocol/vectors/` are replayed against the runtime in `conformance_runtime.rs`.

To add a new vector:
1. Create `packages/ssma-protocol/vectors/my_scenario.json`
2. Add a test in `tests/conformance_runtime.rs` that replays it
3. Assert the emitted frames match expected shapes

## Test Isolation

- Always use `tempfile::tempdir()` for file-based state
- Each test gets its own `users.json`, `intents.json`, `media/` directory
- Tests run in parallel by default; use `--test-threads=1` if needed

## Common Assertions

```rust
// HTTP response
assert_eq!(resp.status(), StatusCode::OK);
let body: Value = serde_json::from_slice(&axum::body::to_bytes(resp.into_body(), 8192).await.unwrap()).unwrap();

// JSON structure
assert_eq!(body["status"], "ok");
assert!(body["intents"].is_array());

// Cookie present
let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
assert!(cookie.contains("ssma_session="));
assert!(cookie.contains("HttpOnly"));
```
