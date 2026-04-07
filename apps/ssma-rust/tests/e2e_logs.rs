use axum::body::Body;
use axum::extract::State as AxumState;
use axum::routing::post;
use axum::{Json, Router};
use http::{Request, StatusCode};
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower::ServiceExt;

fn test_config(tmp: &Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config.log_relay_url = String::new(); // disabled by default
    config
}

#[tokio::test]
async fn logs_batch_returns_disabled_when_relay_url_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let body = serde_json::to_string(&json!({
        "logs": [{"level": "info", "message": "test"}],
        "site": "test-site"
    }))
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/logs/batch")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "disabled");
}

#[tokio::test]
async fn logs_health_returns_disabled_when_relay_url_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/logs/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "disabled");
    assert!(result["relayUrl"].is_null());
}

#[tokio::test]
async fn logs_batch_rate_limits_after_60_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let _app = gateway::app(state.clone());

    let log_body = serde_json::to_string(&json!({
        "logs": [{"level": "info", "message": "test"}],
        "site": "test-site"
    }))
    .unwrap();

    // Send 60 requests - all should succeed
    for _ in 0..60 {
        let req = Request::builder()
            .method("POST")
            .uri("/logs/batch")
            .header("content-type", "application/json")
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::from(log_body.clone()))
            .unwrap();
        let app = gateway::app(state.clone());
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "request within limit should succeed");
    }

    // 61st request should be rate limited
    let req = Request::builder()
        .method("POST")
        .uri("/logs/batch")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "1.2.3.4")
        .body(Body::from(log_body.clone()))
        .unwrap();
    let app = gateway::app(state.clone());
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS, "request over limit should be 429");

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["error"], "RATE_LIMITED");
}

// --- Toy backend for relay tests ---

#[derive(Clone, Default)]
struct ToyLogBackend {
    received: Arc<Mutex<Vec<Value>>>,
}

async fn toy_logs_batch(
    AxumState(state): AxumState<ToyLogBackend>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.received.lock().unwrap().push(body);
    Json(json!({"status": "ok"}))
}

#[tokio::test]
async fn logs_batch_includes_gateway_metrics_in_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let toy = ToyLogBackend::default();

    // Start toy backend on random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let toy_app = Router::new()
        .route("/logs/batch", post(toy_logs_batch))
        .with_state(toy.clone());
    tokio::spawn(async move {
        axum::serve(listener, toy_app).await.unwrap();
    });

    // Configure gateway with relay URL pointing to toy backend
    let mut config = test_config(tmp.path());
    config.log_relay_url = format!("http://127.0.0.1:{}/logs/batch", port);
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let log_body = serde_json::to_string(&json!({
        "logs": [{"level": "info", "message": "hello"}],
        "site": "test-site"
    }))
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/logs/batch")
        .header("content-type", "application/json")
        .body(Body::from(log_body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "ok");

    // Wait for relay to complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify toy backend received the batch with gateway metrics
    let received = toy.received.lock().unwrap();
    assert_eq!(received.len(), 1, "toy backend should receive one relayed batch");

    let relayed = &received[0];
    // Original payload preserved
    assert_eq!(relayed["logs"][0]["level"], "info");
    assert_eq!(relayed["site"], "test-site");

    // Gateway metrics entry present
    let gateway = &relayed["gateway"];
    assert!(gateway["timestamp"].is_number(), "gateway.timestamp should be a number");
    assert!(gateway["metrics"].is_object(), "gateway.metrics should be an object");
    assert!(gateway["metrics"]["active"]["ws"].is_number());
    assert!(gateway["metrics"]["active"]["sse"].is_number());
    assert!(gateway["metrics"]["totals"]["wsConnections"].is_number());
    assert!(gateway["metrics"]["totals"]["sseConnections"].is_number());
    assert!(gateway["metrics"]["totals"]["broadcasts"].is_number());
    assert!(gateway["metrics"]["totals"]["rateLimitHits"].is_number());
    assert!(gateway["metrics"]["serverEvents"].is_object());
}

#[tokio::test]
async fn logs_batch_accepts_csma_batch_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let toy = ToyLogBackend::default();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let toy_app = Router::new()
        .route("/logs/batch", post(toy_logs_batch))
        .with_state(toy.clone());
    tokio::spawn(async move {
        axum::serve(listener, toy_app).await.unwrap();
    });

    let mut config = test_config(tmp.path());
    config.log_relay_url = format!("http://127.0.0.1:{}/logs/batch", port);
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let body = serde_json::to_string(&json!({
        "batchId": "csma-batch-1",
        "sessionId": "session-1",
        "userId": "anonymous",
        "source": "csma",
        "meta": {
            "appVersion": "1.2.0",
            "platform": "web"
        },
        "entries": [
            {
                "event": "ANALYTICS_EVENT_CHECKOUT",
                "level": "info",
                "message": "Event: checkout",
                "tags": ["analytics"],
                "context": {
                    "sessionId": "session-1"
                },
                "timestamp": 123
            }
        ]
    }))
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/logs/batch")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["forwarded"], 1);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let received = toy.received.lock().unwrap();
    assert_eq!(received.len(), 1);

    let relayed = &received[0];
    assert_eq!(relayed["batchId"], "csma-batch-1");
    assert_eq!(relayed["sessionId"], "session-1");
    assert_eq!(relayed["userId"], "anonymous");
    assert_eq!(relayed["source"], "csma");
    assert_eq!(relayed["meta"]["appVersion"], "1.2.0");
    assert_eq!(relayed["entries"][0]["event"], "ANALYTICS_EVENT_CHECKOUT");
    assert!(relayed["gateway"]["metrics"].is_object());
}

#[tokio::test]
async fn logs_health_reports_active_when_relay_configured() {
    let tmp = tempfile::tempdir().unwrap();
    let toy = ToyLogBackend::default();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let toy_app = Router::new()
        .route("/logs/batch", post(toy_logs_batch))
        .with_state(toy);
    tokio::spawn(async move {
        axum::serve(listener, toy_app).await.unwrap();
    });

    let mut config = test_config(tmp.path());
    config.log_relay_url = format!("http://127.0.0.1:{}/logs/batch", port);
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/logs/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "active");
    assert!(result["relayUrl"].is_string());
    assert!(result["relayUrl"].as_str().unwrap().contains(&format!("{}", port)));
}
