use anyhow::Result;
use axum::extract::State as AxumState;
use axum::http::HeaderMap;
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
struct BackendState {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

#[derive(Clone, Default)]
struct VerifyState {
    response: Arc<Mutex<Value>>,
}

fn test_config(tmp: &StdPath, backend_url: String) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config.backend_url = backend_url;
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

async fn backend_ingest(
    AxumState(state): AxumState<BackendState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    state.calls.lock().expect("calls lock").push((request_id, body));
    Json(json!({"status": "ok"}))
}

async fn spawn_backend() -> Result<(String, BackendState, tokio::task::JoinHandle<()>)> {
    let state = BackendState::default();
    let app = Router::new()
        .route("/webhooks/ingest", post(backend_ingest))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), state, handle))
}

async fn verify(
    AxumState(state): AxumState<VerifyState>,
    Json(_body): Json<Value>,
) -> Json<Value> {
    Json(state.response.lock().expect("verify lock").clone())
}

async fn spawn_verify(response: Value) -> Result<(String, VerifyState, tokio::task::JoinHandle<()>)> {
    let state = VerifyState {
        response: Arc::new(Mutex::new(response)),
    };
    let app = Router::new().route("/verify", post(verify)).with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), state, handle))
}

#[tokio::test]
async fn webhook_disabled_mode_forwards_and_dedupes() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.webhook_verify_mode = "disabled".to_string();
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let client = reqwest::Client::new();
    let request = || {
        client
            .post(format!("http://{}/webhooks/stripe", gateway_base))
            .header("x-request-id", "req-webhook-1")
            .json(&json!({
                "eventId": "evt_1",
                "eventType": "payment.succeeded",
                "payload": {"amount": 100}
            }))
    };

    let first = request().send().await?;
    assert_eq!(first.status(), StatusCode::OK);
    let second = request().send().await?;
    assert_eq!(second.status(), StatusCode::ACCEPTED);

    let calls = backend_state.calls.lock().expect("calls lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "req-webhook-1");
    assert_eq!(calls[0].1["provider"], "stripe");
    assert_eq!(calls[0].1["eventId"], "evt_1");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn webhook_external_verify_failure_blocks() -> Result<()> {
    let (backend_url, backend_state, backend_handle) = spawn_backend().await?;
    let (verify_url, _verify_state, verify_handle) =
        spawn_verify(json!({"ok": false})).await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path(), backend_url);
    config.webhook_verify_mode = "external".to_string();
    config.webhook_verify_url = format!("{}/verify", verify_url);
    let (gateway_base, gateway_handle) = spawn_gateway(config).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/webhooks/stripe", gateway_base))
        .json(&json!({"foo": "bar"}))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"], "WEBHOOK_VERIFICATION_FAILED");
    assert_eq!(backend_state.calls.lock().expect("calls lock").len(), 0);

    gateway_handle.abort();
    verify_handle.abort();
    backend_handle.abort();
    Ok(())
}
