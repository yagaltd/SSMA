use anyhow::Result;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Default)]
struct ToyBackendState {
    seen: Arc<Mutex<HashSet<String>>>,
    apply_count: Arc<Mutex<HashMap<String, usize>>>,
}

async fn toy_apply(
    State(state): State<ToyBackendState>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let intents = body.get("intents").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut results = Vec::new();
    let mut events = Vec::new();
    for intent in intents {
        let id = intent.get("id").and_then(|v| v.as_str()).unwrap_or("missing-id").to_string();
        {
            let mut count = state.apply_count.lock().expect("apply_count lock");
            *count.entry(id.clone()).or_insert(0) += 1;
        }
        let seen_before = {
            let mut seen = state.seen.lock().expect("seen lock");
            !seen.insert(id.clone())
        };
        if seen_before {
            results.push(json!({"id": id, "status": "acked", "code": "IDEMPOTENT_REPLAY"}));
            continue;
        }
        let event = json!({
            "eventId": format!("evt-{}", id),
            "reason": "backend-apply",
            "site": "default",
            "timestamp": now_millis(),
            "intents": [intent.clone()]
        });
        events.push(event.clone());
        results.push(json!({"id": id, "status": "acked", "events": [event]}));
    }
    Json(json!({"results": results, "events": events}))
}

async fn toy_metrics(State(state): State<ToyBackendState>) -> Json<Value> {
    let rows = state
        .apply_count
        .lock()
        .expect("apply_count lock")
        .iter()
        .map(|(id, count)| json!({"id": id, "count": count}))
        .collect::<Vec<_>>();
    Json(json!({"status":"ok","applyCountByIntent":rows}))
}

async fn toy_query(Path(name): Path<String>) -> Json<Value> {
    if name == "todos" {
        return Json(json!({"status":"ok","data":{"todos":[]}}));
    }
    Json(json!({"error":"UNKNOWN_QUERY"}))
}

async fn toy_subscribe() -> Json<Value> {
    Json(json!({"status":"ok","snapshot":[],"cursor":0}))
}

async fn toy_health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}

async fn spawn_toy_backend() -> Result<(String, tokio::task::JoinHandle<()>)> {
    let state = ToyBackendState::default();
    let app = Router::new()
        .route("/apply-intents", post(toy_apply))
        .route("/metrics", get(toy_metrics))
        .route("/query/:name", post(toy_query))
        .route("/subscribe", post(toy_subscribe))
        .route("/health", get(toy_health).post(toy_health))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), handle))
}

async fn spawn_gateway_with(
    backend_url: String,
    require_auth: bool,
    configure: impl FnOnce(&mut ssma_rust::runtime::Config),
) -> Result<(String, tokio::task::JoinHandle<()>)> {
    let mut config = ssma_rust::runtime::Config::from_env();
    config.host = "127.0.0.1".to_string();
    config.port = 0;
    config.backend_url = backend_url;
    config.require_auth_for_writes = require_auth;
    config.intent_store_path = std::env::temp_dir().join(format!(
        "ssma-rust-e2e-intents-{}.json",
        uuid::Uuid::new_v4()
    ));
    configure(&mut config);

    let state = ssma_rust::gateway::build_state(config);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = ssma_rust::gateway::app(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("127.0.0.1:{}", addr.port()), handle))
}

async fn spawn_gateway(backend_url: String, require_auth: bool) -> Result<(String, tokio::task::JoinHandle<()>)> {
    spawn_gateway_with(backend_url, require_auth, |_| {}).await
}

async fn ws_wait_for(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ty: &str,
) -> Result<Value> {
    let mut seen = Vec::new();
    let val = timeout(Duration::from_secs(6), async {
        while let Some(msg) = ws.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let value: Value = serde_json::from_str(&text)?;
                if let Some(t) = value.get("type").and_then(|v| v.as_str()) {
                    seen.push(t.to_string());
                }
                if value.get("type").and_then(|v| v.as_str()) == Some(ty) {
                    return Ok::<Value, anyhow::Error>(value);
                }
            }
        }
        anyhow::bail!("message {} not found", ty)
    })
    .await
    .map_err(|_| anyhow::anyhow!("timeout waiting for {}, seen={:?}", ty, seen))??;
    Ok(val)
}

async fn sse_wait_for(base: &str, wanted: &[&str]) -> Result<Value> {
    let response = reqwest::Client::new()
        .get(format!("http://{}/optimistic/events", base))
        .send()
        .await?;
    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);

    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("SSE event not observed before deadline");
        }
        let next = tokio::time::timeout(Duration::from_millis(500), stream.next()).await?;
        let Some(chunk_result) = next else { break };
        let chunk = chunk_result?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(split) = buf.find("\n\n") {
            let frame = buf[..split].to_string();
            buf = buf[split + 2..].to_string();
            let mut ty = "message".to_string();
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(x) = line.strip_prefix("event:") {
                    ty = x.trim().to_string();
                }
                if let Some(x) = line.strip_prefix("data:") {
                    data.push_str(x.trim());
                }
            }
            if wanted.contains(&ty.as_str()) {
                let parsed = serde_json::from_str::<Value>(&data).unwrap_or_else(|_| json!({"raw": data}));
                return Ok(json!({"type": ty, "data": parsed}));
            }
        }
    }
    anyhow::bail!("SSE event not observed")
}

#[tokio::test]
async fn scenarios_a_to_f() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle) = spawn_gateway(backend_base.clone(), false).await?;

    // A + F
    let (mut ws, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0", gateway_base)).await?;
    let _ = ws_wait_for(&mut ws, "hello").await?;
    let _ = ws_wait_for(&mut ws, "replay").await?;

    let (mut mismatch, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=2.0.0", gateway_base)).await?;
    let mismatch_error = ws_wait_for(&mut mismatch, "error").await?;
    assert_eq!(mismatch_error["code"], "SUBPROTOCOL_MISMATCH");

    // B/C
    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-1-abcdefg",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-1","title":"one"},
                "meta": {"clock": now_millis(), "channels": ["global"]}
            }]
        }).to_string(),
    ))
    .await?;
    let ack = ws_wait_for(&mut ws, "ack").await?;
    assert_eq!(ack["intents"][0]["id"], "i-1-abcdefg");
    assert_eq!(ack["intents"][0]["status"], "acked");

    // retry same intent id
    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-1-abcdefg",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-1","title":"one"},
                "meta": {"clock": now_millis(), "channels": ["global"]}
            }]
        }).to_string(),
    ))
    .await?;
    let retry_ack = ws_wait_for(&mut ws, "ack").await?;
    assert_eq!(retry_ack["intents"][0]["status"], "acked");

    let metrics = reqwest::get(format!("{}/metrics", backend_base)).await?.json::<Value>().await?;
    let count = metrics
        .get("applyCountByIntent")
        .and_then(|v| v.as_array())
        .and_then(|rows| rows.iter().find(|r| r.get("id") == Some(&json!("i-1-abcdefg"))))
        .and_then(|r| r.get("count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(count, 1);

    // SSE invalidate observed (subscribe first, then trigger fresh write)
    let gateway_base_for_sse = gateway_base.clone();
    let sse_task = tokio::spawn(async move {
        sse_wait_for(&gateway_base_for_sse, &["invalidate", "island.invalidate"]).await
    });
    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-2-abcdefg",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-2","title":"two"},
                "meta": {"clock": now_millis(), "channels": ["global"]}
            }]
        }).to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "ack").await?;
    let sse = sse_task.await??;
    assert!(
        sse["type"] == "invalidate" || sse["type"] == "island.invalidate",
        "expected invalidate-like event"
    );

    // D unauthorized when auth required (separate gateway instance)
    let (auth_gateway_base, auth_gateway_handle) = spawn_gateway(backend_base.clone(), true).await?;
    let (mut unauth, _) = connect_async(format!("ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0", auth_gateway_base)).await?;
    let _ = ws_wait_for(&mut unauth, "hello").await?;
    let _ = ws_wait_for(&mut unauth, "replay").await?;
    unauth.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-unauth-0001",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-x"},
                "meta": {"clock": 1}
            }]
        }).to_string()
    )).await?;
    let unauth_error = ws_wait_for(&mut unauth, "error").await?;
    assert_eq!(unauth_error["code"], "UNAUTHORIZED");

    // E channel snapshot
    ws.send(Message::Text(
        json!({ "type": "channel.subscribe", "channel": "global", "params": {} }).to_string(),
    ))
    .await?;
    let sub_ack = ws_wait_for(&mut ws, "channel.ack").await?;
    assert_eq!(sub_ack["status"], "ok");
    let snapshot = ws_wait_for(&mut ws, "channel.snapshot").await?;
    assert_eq!(snapshot["channel"], "global");

    // channel.invalidate fanout for subscribed channel
    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-3-abcdefg",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-3","title":"three"},
                "meta": {"clock": now_millis(), "channels": ["global"]}
            }]
        }).to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "ack").await?;
    let invalidate = ws_wait_for(&mut ws, "channel.invalidate").await?;
    assert_eq!(invalidate["type"], "channel.invalidate");

    // observability endpoint
    let gateway_metrics = reqwest::get(format!("http://{}/optimistic/metrics", gateway_base))
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(gateway_metrics["status"], "ok");
    assert!(gateway_metrics["totals"]["broadcasts"].as_u64().unwrap_or(0) >= 1);
    assert!(gateway_metrics["serverEvents"]["CHANNEL_SUBSCRIBE"].as_u64().unwrap_or(0) >= 1);
    assert!(gateway_metrics["serverEvents"]["INTENT_ACKED"].as_u64().unwrap_or(0) >= 1);

    // RBAC deny on protected channels
    let (rbac_gateway_base, rbac_gateway_handle) = spawn_gateway_with(backend_base.clone(), false, |config| {
        config.protected_channels = vec!["admin-only".to_string()];
        config.protected_channel_min_role = "admin".to_string();
    })
    .await?;
    let (mut rbac_ws, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0",
        rbac_gateway_base
    ))
    .await?;
    let _ = ws_wait_for(&mut rbac_ws, "hello").await?;
    let _ = ws_wait_for(&mut rbac_ws, "replay").await?;
    rbac_ws
        .send(Message::Text(
            json!({ "type": "channel.subscribe", "channel": "admin-only", "params": {} }).to_string(),
        ))
        .await?;
    let denied = ws_wait_for(&mut rbac_ws, "channel.ack").await?;
    assert_eq!(denied["code"], "ACCESS_DENIED");
    let close = ws_wait_for(&mut rbac_ws, "channel.close").await?;
    assert_eq!(close["code"], "ACCESS_DENIED");

    rbac_gateway_handle.abort();
    auth_gateway_handle.abort();
    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

fn now_millis() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
