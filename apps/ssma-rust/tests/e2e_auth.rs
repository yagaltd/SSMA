use axum::body::Body;
use axum::extract::State as AxumState;
use axum::routing::post;
use axum::{Json, Router};
use http::header::{COOKIE, SET_COOKIE};
use http::{Request, StatusCode};
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower::ServiceExt;

#[derive(Clone, Default)]
struct OutboxState {
    events: Arc<Mutex<Vec<Value>>>,
}

fn test_config(tmp: &Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config
}

async fn spawn_outbox_backend() -> (String, OutboxState, tokio::task::JoinHandle<()>) {
    async fn outbox(
        AxumState(state): AxumState<OutboxState>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.events.lock().expect("outbox lock").push(body);
        Json(json!({ "status": "ok" }))
    }

    let state = OutboxState::default();
    let app = Router::new()
        .route("/auth/outbox", post(outbox))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backend");
    let addr = listener.local_addr().expect("backend addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://127.0.0.1:{}", addr.port()), state, handle)
}

fn extract_cookie(response: &http::Response<Body>, cookie_name: &str) -> Option<String> {
    response.headers().get_all(SET_COOKIE).iter().find_map(|v| {
        let s = v.to_str().ok()?;
        if s.starts_with(&format!("{}=", cookie_name)) {
            Some(s.to_string())
        } else {
            None
        }
    })
}

fn cookie_header_value(cookie: &str) -> String {
    cookie.split(';').next().unwrap_or("").trim().to_string()
}

async fn register_user(
    app: &Router,
    email: &str,
    password: &str,
    name: &str,
) -> http::Response<Body> {
    let body = serde_json::to_string(&json!({
        "email": email,
        "password": password,
        "name": name
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn register_returns_201_with_cookie_and_user() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let resp = register_user(&app, "alice@example.com", "password123", "Alice").await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let cookie = extract_cookie(&resp, "ssma_session").expect("should set auth cookie");
    assert!(cookie.contains("ssma_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(!cookie.contains("Secure"));
    assert!(extract_cookie(&resp, "ssma_refresh").is_some());
    assert!(resp.headers().get("x-request-id").is_some());

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let user: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(user["user"]["email"], "alice@example.com");
    assert_eq!(user["user"]["emailVerified"], true);
}

#[tokio::test]
async fn duplicate_email_returns_409() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let first = register_user(&app, "bob@example.com", "password123", "Bob").await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = register_user(&app, "bob@example.com", "different123", "Bob2").await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_correct_password_returns_200_with_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let reg = register_user(&app, "charlie@example.com", "password123", "Charlie").await;
    assert_eq!(reg.status(), StatusCode::CREATED);

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
    assert!(extract_cookie(&resp, "ssma_session").is_some());
    assert!(extract_cookie(&resp, "ssma_refresh").is_some());
    assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let _ = register_user(&app, "dave@example.com", "password123", "Dave").await;

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
    let reg = register_user(&app, "eve@example.com", "password123", "Eve").await;
    let cookie = extract_cookie(&reg, "ssma_session").unwrap();

    let req = Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header(COOKIE, cookie_header_value(&cookie))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("x-request-id").is_some());
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
async fn refresh_rotates_session_and_refresh_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(tmp.path());
    config.auth_refresh_enabled = true;
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let reg = register_user(&app, "rot@example.com", "password123", "Rot").await;
    let refresh_cookie = extract_cookie(&reg, "ssma_refresh").expect("refresh cookie");

    let req = Request::builder()
        .method("POST")
        .uri("/auth/refresh")
        .header(COOKIE, cookie_header_value(&refresh_cookie))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(extract_cookie(&resp, "ssma_session").is_some());
    assert!(extract_cookie(&resp, "ssma_refresh").is_some());
    assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn email_verification_flow_blocks_login_until_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let (backend_url, outbox_state, backend_handle) = spawn_outbox_backend().await;
    let mut config = test_config(tmp.path());
    config.backend_url = backend_url;
    config.auth_require_email_verification = true;
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let reg = register_user(&app, "verify@example.com", "password123", "Verify").await;
    assert_eq!(reg.status(), StatusCode::CREATED);

    let login_body = serde_json::to_string(&json!({
        "email": "verify@example.com",
        "password": "password123"
    }))
    .unwrap();
    let login_req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(login_body))
        .unwrap();
    let blocked = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    let resend_req = Request::builder()
        .method("POST")
        .uri("/auth/resend-verification")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "email": "verify@example.com" })).unwrap(),
        ))
        .unwrap();
    let resend_resp = app.clone().oneshot(resend_req).await.unwrap();
    assert_eq!(resend_resp.status(), StatusCode::OK);

    let token = {
        let events = outbox_state.events.lock().expect("outbox lock");
        assert!(
            events
                .iter()
                .filter(|e| e["kind"] == "verify_email")
                .count()
                >= 2
        );
        let event = events
            .iter()
            .rev()
            .find(|e| e["kind"] == "verify_email")
            .expect("verify email event");
        event["payload"]["token"].as_str().unwrap().to_string()
    };

    let verify_req = Request::builder()
        .method("POST")
        .uri("/auth/verify-email")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "token": token })).unwrap(),
        ))
        .unwrap();
    let verify_resp = app.clone().oneshot(verify_req).await.unwrap();
    assert_eq!(verify_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(verify_resp.into_body(), 8192)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["user"]["emailVerified"], true);

    let login_req2 = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "email": "verify@example.com",
                "password": "password123"
            }))
            .unwrap(),
        ))
        .unwrap();
    let ok = app.oneshot(login_req2).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    backend_handle.abort();
}

#[tokio::test]
async fn forgot_and_reset_password_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let (backend_url, outbox_state, backend_handle) = spawn_outbox_backend().await;
    let mut config = test_config(tmp.path());
    config.backend_url = backend_url;
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let _ = register_user(&app, "reset@example.com", "password123", "Reset").await;

    let forgot_req = Request::builder()
        .method("POST")
        .uri("/auth/forgot-password")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "email": "reset@example.com" })).unwrap(),
        ))
        .unwrap();
    let forgot_resp = app.clone().oneshot(forgot_req).await.unwrap();
    assert_eq!(forgot_resp.status(), StatusCode::OK);
    assert!(forgot_resp.headers().get("x-request-id").is_some());

    let reset_token = {
        let events = outbox_state.events.lock().expect("outbox lock");
        let event = events
            .iter()
            .find(|e| e["kind"] == "password_reset")
            .expect("reset event");
        event["payload"]["token"].as_str().unwrap().to_string()
    };

    let reset_req = Request::builder()
        .method("POST")
        .uri("/auth/reset-password")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "token": reset_token,
                "newPassword": "new-password-123"
            }))
            .unwrap(),
        ))
        .unwrap();
    let reset_resp = app.clone().oneshot(reset_req).await.unwrap();
    assert_eq!(reset_resp.status(), StatusCode::OK);

    let old_login = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "email": "reset@example.com",
                "password": "password123"
            }))
            .unwrap(),
        ))
        .unwrap();
    let old_login_resp = app.clone().oneshot(old_login).await.unwrap();
    assert_eq!(old_login_resp.status(), StatusCode::UNAUTHORIZED);

    let new_login = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "email": "reset@example.com",
                "password": "new-password-123"
            }))
            .unwrap(),
        ))
        .unwrap();
    let new_login_resp = app.oneshot(new_login).await.unwrap();
    assert_eq!(new_login_resp.status(), StatusCode::OK);

    backend_handle.abort();
}
