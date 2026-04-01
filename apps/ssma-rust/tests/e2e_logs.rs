use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path;
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
