use axum::body::Body;
use http::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
    ORIGIN,
};
use http::{Request, StatusCode};
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
    config.allowed_origins = "*".to_string(); // Enable CORS for tests
    config
}

#[tokio::test]
async fn preflight_options_returns_cors_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("OPTIONS")
        .uri("/health")
        .header(ORIGIN, "http://localhost:3000")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let allow_origin = resp
        .headers()
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .expect("should have access-control-allow-origin header");
    // very_permissive reflects the requesting origin
    assert_eq!(allow_origin.to_str().unwrap(), "http://localhost:3000");
}

#[tokio::test]
async fn actual_request_includes_cors_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .header(ORIGIN, "http://localhost:3000")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let allow_origin = resp
        .headers()
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .expect("should have access-control-allow-origin header");
    // very_permissive reflects the requesting origin
    assert_eq!(allow_origin.to_str().unwrap(), "http://localhost:3000");
}

#[tokio::test]
async fn specific_origins_config_restricts_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.allowed_origins = "http://allowed.example.com,http://other.example.com".to_string();
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    // Request with a non-allowed origin should not get the origin reflected back
    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .header(ORIGIN, "http://not-allowed.example.com")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // With specific origins configured, non-matching origin should not be reflected
    let allow_origin = resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN);
    // tower-http with specific origins will either not set the header or reject the origin
    assert!(
        allow_origin.is_none()
            || allow_origin.unwrap().to_str().unwrap() != "http://not-allowed.example.com",
        "non-allowed origin should not be reflected in CORS header"
    );
}

#[tokio::test]
async fn specific_origins_config_allows_matching_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.allowed_origins = "http://allowed.example.com,http://other.example.com".to_string();
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .header(ORIGIN, "http://allowed.example.com")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let allow_origin = resp
        .headers()
        .get(ACCESS_CONTROL_ALLOW_ORIGIN)
        .expect("should have access-control-allow-origin header for allowed origin");
    assert_eq!(
        allow_origin.to_str().unwrap(),
        "http://allowed.example.com"
    );
}
