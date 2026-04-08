use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::Value;
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
    config
}

#[tokio::test]
async fn health_returns_200_with_status() {
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

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert!(json["subprotocol"].is_string());
}

#[tokio::test]
async fn health_includes_cursor() {
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

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["cursor"].is_number());
    assert_eq!(json["service"], "ssma-rust");
}

#[tokio::test]
async fn ready_returns_200_when_backend_unconfigured() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.backend_url = "".to_string(); // No backend configured
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/ready")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["backend"], "unconfigured");
}

#[tokio::test]
async fn ready_includes_backend_status() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.backend_url = "".to_string(); // No backend configured
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/ready")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["backend"].is_string());
    assert!(json["cursor"].is_number());
}

#[tokio::test]
async fn ready_includes_subprotocol() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/ready")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["subprotocol"].is_string());
}

#[tokio::test]
async fn config_validation_rejects_insecure_jwt_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.auth_jwt_secret = "change-me-in-production".to_string();
    config.user_store_path = tmp.path().join("users.json");
    config.intent_store_path = tmp.path().join("intents.json");
    config.media_storage_root = tmp.path().join("media");

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("SSMA_AUTH_JWT_SECRET"));
}

#[tokio::test]
async fn config_validation_succeeds_with_valid_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.auth_jwt_secret = "valid-secret-key".to_string();
    config.user_store_path = tmp.path().join("users.json");
    config.intent_store_path = tmp.path().join("intents.json");
    config.media_storage_root = tmp.path().join("media");

    let result = config.validate();
    assert!(result.is_ok());
}

#[tokio::test]
async fn config_validation_checks_path_writability() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.auth_jwt_secret = "valid-secret-key".to_string();
    config.user_store_path = tmp.path().join("users.json");
    config.intent_store_path = tmp.path().join("intents.json");
    config.media_storage_root = tmp.path().join("media");

    let result = config.validate();
    assert!(result.is_ok());
    
    // Verify paths were created
    assert!(tmp.path().join("users.json").exists());
    assert!(tmp.path().join("intents.json").exists());
    assert!(tmp.path().join("media").exists());
}
