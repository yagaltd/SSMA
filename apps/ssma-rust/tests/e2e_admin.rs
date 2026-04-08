use http::StatusCode;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use serde_json::json;
use ssma_rust::config::Config;
use ssma_rust::runtime::IntentRecord;
use std::path::Path;
use tokio::net::TcpListener;

fn test_config(tmp: &Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config
}

#[derive(Serialize)]
struct Claims {
    sub: String,
    role: String,
    iss: String,
    aud: String,
    iat: u64,
    exp: u64,
}

fn make_cookie(config: &Config, user_id: &str, role: &str) -> String {
    let now = ssma_rust::runtime::now_secs();
    let claims = Claims {
        sub: user_id.to_string(),
        role: role.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.jwt_audience.clone(),
        iat: now,
        exp: now + 3600,
    };
    let jwt = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.auth_jwt_secret.as_bytes()),
    )
    .unwrap();
    format!("ssma_session={}", jwt)
}

fn make_intent(id: &str, reasons: &[&str]) -> IntentRecord {
    IntentRecord {
        id: id.to_string(),
        intent: "cart.add".to_string(),
        payload: json!({}),
        meta: json!({
            "clock": 1,
            "channels": ["global"],
            "reasons": reasons,
        }),
        inserted_at: ssma_rust::runtime::now_millis(),
        log_seq: 0,
        site: "site-a".to_string(),
        status: "acked".to_string(),
        connection_id: Some("conn-1".to_string()),
        actor_key: Some("user:user-1".to_string()),
        user_id: Some("user-1".to_string()),
        backend: None,
    }
}

async fn spawn_server(config: Config) -> (String, tokio::task::JoinHandle<()>) {
    let state = ssma_rust::gateway::build_state(config);
    let app = ssma_rust::gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://127.0.0.1:{}", addr.port()), handle)
}

#[tokio::test]
async fn admin_channels_returns_401_for_unauthenticated() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (base, handle) = spawn_server(config).await;

    let resp = reqwest::get(format!("{}/admin/optimistic/channels", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    handle.abort();
}

#[tokio::test]
async fn admin_intents_returns_403_for_regular_user() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let user_cookie = make_cookie(&config, "user-1", "user");
    let (base, handle) = spawn_server(config).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/admin/optimistic/intents", base))
        .header(reqwest::header::COOKIE, user_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    handle.abort();
}

#[tokio::test]
async fn admin_channels_returns_subscriber_rows_for_staff() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let staff_cookie = make_cookie(&config, "staff-1", "staff");
    let user_cookie = make_cookie(&config, "user-42", "user");
    let (base, handle) = spawn_server(config.clone()).await;

    let mut request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        format!(
            "ws://127.0.0.1:{}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0",
            base.rsplit(':').next().unwrap()
        ),
    )
    .unwrap();
    request
        .headers_mut()
        .insert("Cookie", user_cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let _ = futures_util::StreamExt::next(&mut ws).await;
    let _ = futures_util::StreamExt::next(&mut ws).await;
    futures_util::SinkExt::send(
        &mut ws,
        tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "type": "channel.subscribe",
                "channel": "global",
                "params": { "scope": "all" }
            })
            .to_string()
            .into(),
        ),
    )
    .await
    .unwrap();
    let _ = futures_util::StreamExt::next(&mut ws).await;
    let _ = futures_util::StreamExt::next(&mut ws).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/admin/optimistic/channels", base))
        .header(reqwest::header::COOKIE, staff_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.json::<serde_json::Value>().await.unwrap();

    assert!(body["updatedAt"].is_number());
    assert_eq!(body["totalSubscriptions"], 1);
    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels[0]["channel"], "global");
    assert_eq!(channels[0]["total"], 1);
    let subscriber = &channels[0]["subscribers"][0];
    assert!(subscriber["connectionId"].is_string());
    assert_eq!(subscriber["params"], json!({ "scope": "all" }));
    assert_eq!(subscriber["connectionRole"], "follower");
    assert_eq!(subscriber["site"], "default");
    assert_eq!(subscriber["user"]["id"], "user-42");
    assert_eq!(subscriber["user"]["role"], "user");

    handle.abort();
}

#[tokio::test]
async fn admin_intents_returns_pending_rows_and_reason_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let staff_cookie = make_cookie(&config, "staff-1", "staff");
    let state = ssma_rust::gateway::build_state(config.clone());
    state.store.append_batch(vec![
        make_intent("i1", &["pending", "rework"]),
        make_intent("i2", &["pending"]),
    ]);

    let app = ssma_rust::gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base = format!("http://127.0.0.1:{}", addr.port());

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/admin/optimistic/intents?reason=rework", base))
        .header(reqwest::header::COOKIE, staff_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.json::<serde_json::Value>().await.unwrap();

    assert!(body["updatedAt"].is_number());
    assert_eq!(body["total"], 1);
    assert_eq!(body["pending"][0]["id"], "i1");
    assert_eq!(body["pending"][0]["channels"], json!(["global"]));
    assert_eq!(body["pending"][0]["reasons"], json!(["pending", "rework"]));
    assert_eq!(body["reasonSummary"][0]["reason"], "pending");

    handle.abort();
}

#[tokio::test]
async fn admin_intents_limit_is_capped() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let staff_cookie = make_cookie(&config, "staff-1", "staff");
    let state = ssma_rust::gateway::build_state(config.clone());
    let entries: Vec<_> = (0..10)
        .map(|index| make_intent(&format!("i{}", index), &["pending"]))
        .collect();
    state.store.append_batch(entries);

    let app = ssma_rust::gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base = format!("http://127.0.0.1:{}", addr.port());

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/admin/optimistic/intents?limit=3", base))
        .header(reqwest::header::COOKIE, staff_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["pending"].as_array().unwrap().len(), 3);

    handle.abort();
}
