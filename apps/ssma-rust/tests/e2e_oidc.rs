use anyhow::Result;
use axum::extract::State as AxumState;
use axum::routing::{get, post};
use axum::{Json, Router};
use http::StatusCode;
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::collections::HashMap;
use std::path::Path as StdPath;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct OidcProviderState {
    token_calls: Arc<Mutex<Vec<String>>>,
}

fn test_config(tmp: &StdPath) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
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

async fn provider_token(
    AxumState(state): AxumState<OidcProviderState>,
    body: String,
) -> Json<Value> {
    state.token_calls.lock().expect("token lock").push(body);
    Json(json!({
        "access_token": "access-token-1",
        "token_type": "Bearer",
        "expires_in": 3600
    }))
}

async fn provider_userinfo() -> Json<Value> {
    Json(json!({
        "sub": "oidc-user-1",
        "email": "oidc@example.com",
        "name": "OIDC User"
    }))
}

async fn spawn_provider() -> Result<(String, OidcProviderState, tokio::task::JoinHandle<()>)> {
    let state = OidcProviderState::default();
    let app = Router::new()
        .route("/authorize", get(|| async { "ok" }))
        .route("/token", post(provider_token))
        .route("/userinfo", get(provider_userinfo))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), state, handle))
}

#[tokio::test]
async fn oidc_start_redirects_to_provider() -> Result<()> {
    let (provider, _provider_state, provider_handle) = spawn_provider().await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path());
    config.oidc_enabled = true;
    config.oidc_client_id = "client-1".to_string();
    config.oidc_auth_url = format!("{}/authorize", provider);
    config.oidc_token_url = format!("{}/token", provider);
    config.oidc_redirect_url = "http://localhost/callback".to_string();
    let (gateway, gateway_handle) = spawn_gateway(config).await?;

    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = no_redirect
        .get(format!("http://{}/auth/oidc/start", gateway))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(location.contains("/authorize"));
    assert!(location.contains("state="));
    assert!(location.contains("code_challenge="));

    gateway_handle.abort();
    provider_handle.abort();
    Ok(())
}

#[tokio::test]
async fn oidc_callback_exchanges_code_and_sets_session_cookie() -> Result<()> {
    let (provider, provider_state, provider_handle) = spawn_provider().await?;
    let tmp = tempfile::tempdir()?;
    let mut config = test_config(tmp.path());
    config.oidc_enabled = true;
    config.oidc_client_id = "client-1".to_string();
    config.oidc_client_secret = "secret-1".to_string();
    config.oidc_auth_url = format!("{}/authorize", provider);
    config.oidc_token_url = format!("{}/token", provider);
    config.oidc_userinfo_url = format!("{}/userinfo", provider);
    config.oidc_redirect_url = "http://localhost/callback".to_string();
    let (gateway, gateway_handle) = spawn_gateway(config).await?;

    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let start = no_redirect
        .get(format!("http://{}/auth/oidc/start", gateway))
        .send()
        .await?;
    let location = start
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let url = reqwest::Url::parse(&location)?;
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let state = params.get("state").expect("state").to_string();

    let callback = reqwest::Client::new()
        .get(format!(
            "http://{}/auth/oidc/callback?code=fake-code&state={}",
            gateway, state
        ))
        .send()
        .await?;
    assert_eq!(callback.status(), StatusCode::OK);
    let set_cookie = callback
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("ssma_session="))
        .expect("expected ssma_session cookie");
    assert!(set_cookie.contains("HttpOnly"));
    let body: Value = callback.json().await?;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["user"]["email"], "oidc@example.com");

    let token_calls = provider_state.token_calls.lock().expect("token lock");
    assert_eq!(token_calls.len(), 1);
    assert!(token_calls[0].contains("grant_type=authorization_code"));

    gateway_handle.abort();
    provider_handle.abort();
    Ok(())
}
