use anyhow::Result;
use axum::extract::State as AxumState;
use axum::routing::post;
use axum::{Json, Router};
use http::StatusCode;
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path as StdPath;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct FormBackendState {
    submitted: Arc<Mutex<Vec<Value>>>,
}

#[derive(Clone, Default)]
struct CaptchaState {
    calls: Arc<Mutex<Vec<Value>>>,
    ok: bool,
    delay_ms: u64,
}

fn test_config(tmp: &StdPath, backend_url: String) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config.backend_url = backend_url;
    config.form_captcha_mode = "disabled".to_string();
    config.form_rate_window_ms = 60_000;
    config.form_rate_max = 20;
    config
}

async fn spawn_gateway(config: Config) -> Result<(String, tokio::task::JoinHandle<()>)> {
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("127.0.0.1:{}", addr.port()), handle))
}

async fn backend_forms_submit(
    AxumState(state): AxumState<FormBackendState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.submitted.lock().expect("backend lock").push(body);
    Json(json!({"status": "ok", "accepted": true}))
}

async fn spawn_backend() -> Result<(String, FormBackendState, tokio::task::JoinHandle<()>)> {
    let state = FormBackendState::default();
    let app = Router::new()
        .route("/forms/submit", post(backend_forms_submit))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), state, handle))
}

async fn captcha_verify(
    AxumState(state): AxumState<CaptchaState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state.calls.lock().expect("captcha lock").push(body);
    if state.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(state.delay_ms)).await;
    }
    Json(json!({"ok": state.ok}))
}

async fn spawn_captcha(
    ok: bool,
    delay_ms: u64,
) -> Result<(String, CaptchaState, tokio::task::JoinHandle<()>)> {
    let state = CaptchaState {
        calls: Arc::new(Mutex::new(Vec::new())),
        ok,
        delay_ms,
    };
    let app = Router::new()
        .route("/verify", post(captcha_verify))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), state, handle))
}

#[tokio::test]
async fn forms_submit_forwards_when_valid() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let config = test_config(tmp.path(), backend_url);
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": {"email": "user@example.com", "message": "hello"},
            "meta": {"source": "landing"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["accepted"], true);

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["formName"], "contact");
    assert_eq!(submitted[0]["payload"]["email"], "user@example.com");
    assert_eq!(submitted[0]["context"]["site"], "default");
    assert!(submitted[0]["context"]["actorKey"].is_string());

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_honeypot_returns_202_and_does_not_forward() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let config = test_config(tmp.path(), backend_url);
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": {"email": "user@example.com"},
            "honeypot": "i-am-a-bot"
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value = response.json().await?;
    assert_eq!(body["status"], "accepted");
    assert_eq!(body["dropped"], true);
    assert_eq!(body["reason"], "honeypot");

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 0);

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_external_captcha_pass_forwards() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let (captcha_url, captcha_state, captcha_handle) = spawn_captcha(true, 0).await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.form_captcha_mode = "external".to_string();
    config.form_captcha_verify_url = format!("{}/verify", captcha_url);
    config.form_captcha_timeout_ms = 500;
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": {"email": "user@example.com"},
            "captchaToken": "token-ok"
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 1);

    let calls = captcha_state.calls.lock().expect("captcha lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["token"], "token-ok");
    assert_eq!(calls[0]["formName"], "contact");

    gateway_handle.abort();
    captcha_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_external_captcha_fail_blocks() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let (captcha_url, _captcha_state, captcha_handle) = spawn_captcha(false, 0).await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.form_captcha_mode = "external".to_string();
    config.form_captcha_verify_url = format!("{}/verify", captcha_url);
    config.form_captcha_timeout_ms = 500;
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": {"email": "user@example.com"},
            "captchaToken": "token-fail"
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"], "CAPTCHA_VERIFICATION_FAILED");

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 0);

    gateway_handle.abort();
    captcha_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_external_captcha_timeout_blocks() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let (captcha_url, _captcha_state, captcha_handle) = spawn_captcha(true, 250).await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.form_captcha_mode = "external".to_string();
    config.form_captcha_verify_url = format!("{}/verify", captcha_url);
    config.form_captcha_timeout_ms = 50;
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": {"email": "user@example.com"},
            "captchaToken": "token-timeout"
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"], "CAPTCHA_VERIFICATION_FAILED");

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 0);

    gateway_handle.abort();
    captcha_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_rate_limit_enforced() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.form_rate_max = 1;
    config.form_rate_window_ms = 60_000;
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let client = reqwest::Client::new();
    let request = || {
        client
            .post(format!("http://{}/forms/submit", gateway_base))
            .header("content-type", "application/json")
            .header("x-forwarded-for", "1.2.3.4")
            .json(&json!({
                "formName": "contact",
                "payload": {"email": "user@example.com"},
            }))
    };

    let first = request().send().await?;
    assert_eq!(first.status(), StatusCode::OK);

    let second = request().send().await?;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = second.json().await?;
    assert_eq!(body["error"], "RATE_LIMITED");

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 1);

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_invalid_payload_rejected() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let config = test_config(tmp.path(), backend_url);
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({
            "formName": "contact",
            "payload": "should-be-object"
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await?;
    assert_eq!(body["error"], "INVALID_FORM_PAYLOAD");

    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 0);

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_urlencoded_payload_supported() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let config = test_config(tmp.path(), backend_url);
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("formName=contact&email=user%40example.com&message=hello")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let submitted = backend_state.submitted.lock().expect("backend lock");
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["payload"]["email"], "user@example.com");
    assert_eq!(submitted[0]["payload"]["message"], "hello");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn forms_csrf_double_submit_enforced_for_urlencoded() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.form_csrf_mode = "double-submit".to_string();
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let fail = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("formName=contact&email=user%40example.com")
        .send()
        .await?;
    assert_eq!(fail.status(), StatusCode::FORBIDDEN);

    let ok = reqwest::Client::new()
        .post(format!("http://{}/forms/submit", gateway_base))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", "ssma_csrf=token-1")
        .header("x-csrf-token", "token-1")
        .body("formName=contact&email=user%40example.com")
        .send()
        .await?;
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(backend_state.submitted.lock().expect("backend lock").len(), 1);

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}
