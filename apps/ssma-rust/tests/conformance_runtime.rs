use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_gateway(mut config: ssma_rust::runtime::Config) -> Result<(String, tokio::task::JoinHandle<()>)> {
    config.host = "127.0.0.1".to_string();
    config.port = 0;
    config.backend_url = "".to_string();
    let state = ssma_rust::gateway::build_state(config);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = ssma_rust::gateway::app(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("127.0.0.1:{}", addr.port()), handle))
}

async fn ws_wait_for(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ty: &str,
) -> Result<Value> {
    let deadline = Duration::from_secs(12);
    let mut seen = Vec::new();
    let found = timeout(deadline, async {
        while let Some(msg) = ws.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let json: Value = serde_json::from_str(&text)?;
                if let Some(found_ty) = json.get("type").and_then(|v| v.as_str()) {
                    seen.push(found_ty.to_string());
                }
                if json.get("type").and_then(|v| v.as_str()) == Some(ty) {
                    return Ok::<Value, anyhow::Error>(json);
                }
            }
        }
        anyhow::bail!("socket ended before {}", ty)
    })
    .await
    .map_err(|_| anyhow::anyhow!("timeout waiting for {}, seen={:?}", ty, seen))??;
    Ok(found)
}

#[tokio::test]
async fn conformance_vectors_replayed_against_runtime() -> Result<()> {
    let (base, handle) = spawn_gateway(ssma_rust::runtime::Config::from_env()).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ws_handshake
    let (mut ws, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0", base)).await?;
    let hello = ws_wait_for(&mut ws, "hello").await?;
    assert_eq!(hello.get("subprotocol").and_then(|v| v.as_str()), Some("1.0.0"));
    let _ = ws_wait_for(&mut ws, "replay").await?;

    // intent_batch_ack
    ws.send(Message::Text(
        serde_json::json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-1-0001",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-1","title":"one"},
                "meta": {"clock": 1, "channels": ["global"]}
            }]
        })
        .to_string(),
    ))
    .await?;
    let ack = ws_wait_for(&mut ws, "ack").await?;
    assert_eq!(ack["intents"][0]["id"], "i-1-0001");
    assert_eq!(ack["intents"][0]["status"], "acked");

    // channel_subscribe_snapshot
    ws.send(Message::Text(
        serde_json::json!({ "type": "channel.subscribe", "channel": "global", "params": {} }).to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "channel.ack").await?;
    let _ = ws_wait_for(&mut ws, "channel.snapshot").await?;

    // rate_limit_channel_subscribe
    ws.send(Message::Text(
        serde_json::json!({ "type": "channel.subscribe", "channel": "global", "params": {} }).to_string(),
    ))
    .await?;
    let maybe_rate = ws_wait_for(&mut ws, "channel.ack").await?;
    assert!(
        maybe_rate["status"] == "ok" || maybe_rate["code"] == "RATE_LIMITED",
        "expected channel ack or rate limited"
    );

    // unauthorized_ws_reject using auth-required runtime
    handle.abort();
    let mut auth_cfg = ssma_rust::runtime::Config::from_env();
    auth_cfg.require_auth_for_writes = true;
    let (base2, handle2) = spawn_gateway(auth_cfg).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (mut unauth, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0", base2)).await?;
    let _ = ws_wait_for(&mut unauth, "hello").await?;
    let _ = ws_wait_for(&mut unauth, "replay").await?;
    unauth
        .send(Message::Text(
            serde_json::json!({
                "type": "intent.batch",
                "intents": [{
                    "id": "i-unauth",
                    "intent": "TODO_CREATE",
                    "payload": {"id": "todo-x"},
                    "meta": {"clock": 1}
                }]
            })
            .to_string(),
        ))
        .await?;
    let err = ws_wait_for(&mut unauth, "error").await?;
    assert_eq!(err["code"], "UNAUTHORIZED");

    // subprotocol mismatch shape parity
    let (mut mismatch, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=2.0.0", base2)).await?;
    let mismatch_error = ws_wait_for(&mut mismatch, "error").await?;
    assert_eq!(mismatch_error["code"], "SUBPROTOCOL_MISMATCH");

    handle2.abort();
    Ok(())
}
