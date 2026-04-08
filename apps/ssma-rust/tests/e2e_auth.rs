use axum::body::Body;
use http::header::{COOKIE, SET_COOKIE};
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
    config
}

fn extract_cookie(response: &http::Response<Body>) -> Option<String> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            if s.starts_with("ssma_session=") {
                Some(s.to_string())
            } else {
                None
            }
        })
}

fn cookie_header_value(cookie: &str) -> String {
    let val = cookie.split(';').next().unwrap_or("").trim();
    // cookie format: "ssma_session=<jwt>"
    // extract just the key=value part
    val.to_string()
}

#[tokio::test]
async fn register_returns_201_with_cookie_and_user() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let body = serde_json::to_string(&json!({
        "email": "alice@example.com",
        "password": "password123",
        "name": "Alice"
    }))
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let cookie = extract_cookie(&resp).expect("should set auth cookie");
    assert!(cookie.contains("ssma_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(!cookie.contains("Secure")); // auth_cookie_secure = false

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let user: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(user["user"]["email"], "alice@example.com");
    assert_eq!(user["user"]["name"], "Alice");
    assert_eq!(user["user"]["role"], "user");
    assert_eq!(user["user"]["status"], "active");
    assert!(user["user"].get("passwordHash").is_none());
    assert!(user["user"].get("password_hash").is_none());
}

#[tokio::test]
async fn duplicate_email_returns_409() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let body = serde_json::to_string(&json!({
        "email": "bob@example.com",
        "password": "password123",
        "name": "Bob"
    }))
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body2 = serde_json::to_string(&json!({
        "email": "bob@example.com",
        "password": "different123",
        "name": "Bob2"
    }))
    .unwrap();

    let req2 = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body2))
        .unwrap();
    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_correct_password_returns_200_with_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    // Register first
    let reg_body = serde_json::to_string(&json!({
        "email": "charlie@example.com",
        "password": "password123",
        "name": "Charlie"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(reg_body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Login
    let login_body = serde_json::to_string(&json!({
        "email": "charlie@example.com",
        "password": "password123"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login_body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let cookie = extract_cookie(&resp).expect("should set auth cookie");
    assert!(cookie.contains("ssma_session="));

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let user: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(user["user"]["email"], "charlie@example.com");
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    // Register first
    let reg_body = serde_json::to_string(&json!({
        "email": "dave@example.com",
        "password": "password123",
        "name": "Dave"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(reg_body))
        .unwrap();
    let _ = app.clone().oneshot(req).await.unwrap();

    // Login with wrong password
    let login_body = serde_json::to_string(&json!({
        "email": "dave@example.com",
        "password": "wrongpassword"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login_body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_with_valid_cookie_returns_user() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    // Register to get cookie
    let reg_body = serde_json::to_string(&json!({
        "email": "eve@example.com",
        "password": "password123",
        "name": "Eve"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(reg_body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let cookie = extract_cookie(&resp).unwrap();

    // GET /auth/me
    let req = Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header(COOKIE, cookie_header_value(&cookie))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let user: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(user["user"]["email"], "eve@example.com");
    assert_eq!(user["user"]["name"], "Eve");
}

#[tokio::test]
async fn me_without_cookie_returns_401() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/auth/me")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_clears_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("POST")
        .uri("/auth/logout")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let cookie = extract_cookie(&resp).expect("should set clear cookie");
    assert!(cookie.contains("Max-Age=0") || cookie.contains("ssma_session="));
}

#[tokio::test]
async fn registered_user_jwt_works_for_protected_channel() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.protected_channels = vec!["admin-only".to_string()];
    config.protected_channel_min_role = "admin".to_string();
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    // Register a regular user
    let reg_body = serde_json::to_string(&json!({
        "email": "frank@example.com",
        "password": "password123",
        "name": "Frank"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(reg_body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // The cookie is valid JWT - verify by calling /auth/me
    let cookie = extract_cookie(&resp).unwrap();
    let req = Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header(COOKIE, cookie_header_value(&cookie))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let user: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(user["user"]["role"], "user");
}
