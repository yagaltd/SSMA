use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path;
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

fn test_config(tmp: &Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config.backend_url = String::new();
    config
}

async fn spawn_server(config: Config) -> (String, tokio::task::JoinHandle<()>) {
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    // Wait for server to be ready
    tokio::time::sleep(Duration::from_millis(50)).await;
    (format!("127.0.0.1:{}", addr.port()), handle)
}

async fn ws_wait_for_type(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    target_type: &str,
) -> Option<Value> {
    let deadline = Duration::from_secs(10);
    timeout(deadline, async {
        while let Some(msg) = ws.next().await {
            if let Ok(Message::Text(text)) = msg {
                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                    if json.get("type").and_then(|v| v.as_str()) == Some(target_type) {
                        return Some(json);
                    }
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn ws_connects_and_receives_hello() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    let url = format!("ws://{}/optimistic/ws", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let hello = ws_wait_for_type(&mut ws, "hello").await.expect("should receive hello");
    assert_eq!(hello.get("subprotocol").and_then(|v| v.as_str()), Some("1.0.0"));

    handle.abort();
}

#[tokio::test]
async fn ws_receives_replay_after_hello() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    let url = format!("ws://{}/optimistic/ws", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let _hello = ws_wait_for_type(&mut ws, "hello").await.expect("should receive hello");
    let replay = ws_wait_for_type(&mut ws, "replay").await.expect("should receive replay");
    assert!(replay.get("intents").is_some());
    assert!(replay.get("cursor").is_some());

    handle.abort();
}

#[tokio::test]
async fn ws_intent_batch_returns_ack() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    // Must connect as leader to send intents
    let url = format!("ws://{}/optimistic/ws?role=leader", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let _hello = ws_wait_for_type(&mut ws, "hello").await.unwrap();
    let _replay = ws_wait_for_type(&mut ws, "replay").await.unwrap();

    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "ws-test-1",
                "intent": "TEST_ACTION",
                "payload": {"value": 1},
                "meta": {"clock": 1, "channels": ["global"]}
            }]
        })
        .to_string(),
    ))
    .await
    .unwrap();

    // Should receive ack - use longer timeout for backend call
    let deadline = Duration::from_secs(15);
    let result = timeout(deadline, async {
        while let Some(msg) = ws.next().await {
            if let Ok(Message::Text(text)) = msg {
                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("ack") {
                        return json;
                    }
                }
            }
        }
        panic!("socket closed before ack");
    }).await;

    let ack = result.expect("should receive ack within timeout");
    let intents = ack.get("intents").and_then(|v| v.as_array()).unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0]["id"], "ws-test-1");
    assert_eq!(intents[0]["status"], "acked");

    handle.abort();
}

#[tokio::test]
async fn ws_channel_subscribe_returns_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    let url = format!("ws://{}/optimistic/ws", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let _hello = ws_wait_for_type(&mut ws, "hello").await.unwrap();
    let _replay = ws_wait_for_type(&mut ws, "replay").await.unwrap();

    ws.send(Message::Text(
        json!({
            "type": "channel.subscribe",
            "channel": "global",
            "params": {}
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let ack = ws_wait_for_type(&mut ws, "channel.ack").await.expect("should receive channel.ack");
    assert_eq!(ack.get("status").and_then(|v| v.as_str()), Some("ok"));

    let snapshot = ws_wait_for_type(&mut ws, "channel.snapshot").await.expect("should receive channel.snapshot");
    assert_eq!(snapshot.get("channel").and_then(|v| v.as_str()), Some("global"));

    handle.abort();
}

#[tokio::test]
async fn ws_invalid_json_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    let url = format!("ws://{}/optimistic/ws", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let _hello = ws_wait_for_type(&mut ws, "hello").await.unwrap();
    let _replay = ws_wait_for_type(&mut ws, "replay").await.unwrap();

    ws.send(Message::Text("not valid json".to_string()))
        .await
        .unwrap();

    let err = ws_wait_for_type(&mut ws, "error").await.expect("should receive error");
    assert_eq!(err.get("code").and_then(|v| v.as_str()), Some("INVALID_JSON"));

    handle.abort();
}

#[tokio::test]
async fn ws_subprotocol_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let (addr, handle) = spawn_server(config).await;

    let url = format!("ws://{}/optimistic/ws?subprotocol=99.0.0", addr);
    let request = url.into_client_request().unwrap();
    let (mut ws, _) = connect_async(request).await.unwrap();

    let err = ws_wait_for_type(&mut ws, "error").await.expect("should receive error");
    assert_eq!(err.get("code").and_then(|v| v.as_str()), Some("SUBPROTOCOL_MISMATCH"));

    handle.abort();
}
