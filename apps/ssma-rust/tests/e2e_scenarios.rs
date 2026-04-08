use anyhow::Result;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Default)]
struct ToyBackendState {
    seen: Arc<Mutex<HashSet<String>>>,
    apply_count: Arc<Mutex<HashMap<String, usize>>>,
    gateway_base: Arc<Mutex<Option<String>>>,
    backend_token: Arc<Mutex<Option<String>>>,
}

async fn toy_apply(State(state): State<ToyBackendState>, Json(body): Json<Value>) -> Json<Value> {
    let intents = body
        .get("intents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut results = Vec::new();
    let mut events = Vec::new();
    for intent in intents {
        let id = intent
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("missing-id")
            .to_string();
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

async fn toy_query(
    State(state): State<ToyBackendState>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    if name == "todos" {
        return Json(json!({"status":"ok","data":{"todos":[]}}));
    }
    if name == "echo-context" {
        return Json(json!({"status":"ok","data": body}));
    }
    if name == "create-output-asset" {
        let gateway_base = state
            .gateway_base
            .lock()
            .expect("gateway base lock")
            .clone();
        let backend_token = state
            .backend_token
            .lock()
            .expect("backend token lock")
            .clone();

        let Some(gateway_base) = gateway_base else {
            return Json(json!({"error":"MISSING_GATEWAY_BASE"}));
        };
        let Some(backend_token) = backend_token else {
            return Json(json!({"error":"MISSING_BACKEND_TOKEN"}));
        };

        let site = body
            .get("context")
            .and_then(|value| value.get("site"))
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        let actor_key = body
            .get("context")
            .and_then(|value| value.get("actorKey"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(actor_key) = actor_key else {
            return Json(json!({"error":"MISSING_ACTOR_KEY"}));
        };

        let payload = body.get("payload").cloned().unwrap_or_else(|| json!({}));
        let file_name = payload
            .get("fileName")
            .and_then(Value::as_str)
            .unwrap_or("speech.wav")
            .to_string();
        let mime_type = payload
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("audio/wav")
            .to_string();
        let media_type = payload
            .get("mediaType")
            .and_then(Value::as_str)
            .unwrap_or("audio")
            .to_string();
        let bytes = payload
            .get("content")
            .and_then(Value::as_str)
            .map(|value| value.as_bytes().to_vec())
            .unwrap_or_else(|| b"RIFFgenerated-audio".to_vec());

        let form = reqwest::multipart::Form::new()
            .text("site", site)
            .text("actorKey", actor_key)
            .text("mediaType", media_type)
            .text("mimeType", mime_type.clone())
            .text("fileName", file_name.clone())
            .part(
                "file",
                reqwest::multipart::Part::bytes(bytes)
                    .file_name(file_name)
                    .mime_str(&mime_type)
                    .expect("mime"),
            );
        let client = reqwest::Client::new();
        let response = match client
            .post(format!("http://{}/internal/assets", gateway_base))
            .header("x-ssma-backend-token", backend_token)
            .multipart(form)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return Json(json!({"error": error.to_string()})),
        };
        let status = response.status();
        let json = match response.json::<Value>().await {
            Ok(json) => json,
            Err(error) => return Json(json!({"error": error.to_string()})),
        };
        if !status.is_success() {
            return Json(json!({"error":"INTERNAL_ASSET_CREATE_FAILED","detail":json}));
        }
        return Json(json!({"status":"ok","data": json["asset"].clone()}));
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

async fn spawn_toy_backend_with_state(
    state: ToyBackendState,
) -> Result<(String, tokio::task::JoinHandle<()>)> {
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

async fn spawn_gateway(
    backend_url: String,
    require_auth: bool,
) -> Result<(String, tokio::task::JoinHandle<()>)> {
    spawn_gateway_with(backend_url, require_auth, |_| {}).await
}

async fn ws_wait_for(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ty: &str,
) -> Result<Value> {
    ws_wait_for_with_timeout(ws, ty, Duration::from_secs(6)).await
}

async fn ws_wait_for_with_timeout(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ty: &str,
    wait_for: Duration,
) -> Result<Value> {
    let mut seen = Vec::new();
    let val = timeout(wait_for, async {
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
    sse_wait_for_with_headers_timeout(base, wanted, None, None, Duration::from_secs(8)).await
}

async fn sse_wait_for_with_headers_timeout(
    base: &str,
    wanted: &[&str],
    query: Option<&str>,
    cookie: Option<&str>,
    wait_for: Duration,
) -> Result<Value> {
    let mut request = reqwest::Client::new().get(format!(
        "http://{}/optimistic/events{}",
        base,
        query.unwrap_or("")
    ));
    if let Some(value) = cookie {
        request = request.header("Cookie", value);
    }
    let response = request.send().await?;
    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + wait_for;

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
                let parsed =
                    serde_json::from_str::<Value>(&data).unwrap_or_else(|_| json!({"raw": data}));
                return Ok(json!({"type": ty, "data": parsed}));
            }
        }
    }
    anyhow::bail!("SSE event not observed")
}

#[derive(Serialize)]
struct TestClaims<'a> {
    sub: &'a str,
    role: &'a str,
    exp: usize,
    iat: usize,
}

fn issue_session_cookie(secret: &str, user_id: &str, role: &str) -> Result<String> {
    let now = (now_millis() / 1000) as usize;
    let token = encode(
        &Header::default(),
        &TestClaims {
            sub: user_id,
            role,
            exp: now + 3600,
            iat: now,
        },
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok(format!("ssma_session={}", token))
}

fn extract_cookie(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let prefix = format!("{}=", name);
            value
                .split(';')
                .next()
                .filter(|segment| segment.starts_with(&prefix))
                .map(|segment| segment.to_string())
        })
}

async fn connect_with_cookie(
    url: String,
    cookie: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let mut request = url.into_client_request()?;
    if let Some(value) = cookie {
        request.headers_mut().insert("Cookie", value.parse()?);
    }
    let (ws, _) = connect_async(request).await?;
    Ok(ws)
}

#[tokio::test]
async fn scenarios_a_to_f() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle) = spawn_gateway(backend_base.clone(), false).await?;

    // A + F
    let (mut ws, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0",
        gateway_base
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "hello").await?;
    let _ = ws_wait_for(&mut ws, "replay").await?;

    let (mut mismatch, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=2.0.0",
        gateway_base
    ))
    .await?;
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
        })
        .to_string(),
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
        })
        .to_string(),
    ))
    .await?;
    let retry_ack = ws_wait_for(&mut ws, "ack").await?;
    assert_eq!(retry_ack["intents"][0]["status"], "acked");

    let metrics = reqwest::get(format!("{}/metrics", backend_base))
        .await?
        .json::<Value>()
        .await?;
    let count = metrics
        .get("applyCountByIntent")
        .and_then(|v| v.as_array())
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.get("id") == Some(&json!("i-1-abcdefg")))
        })
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
        })
        .to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "ack").await?;
    let sse = sse_task.await??;
    assert!(
        sse["type"] == "invalidate" || sse["type"] == "island.invalidate",
        "expected invalidate-like event"
    );

    // D unauthorized when auth required (separate gateway instance)
    let (auth_gateway_base, auth_gateway_handle) =
        spawn_gateway(backend_base.clone(), true).await?;
    let (mut unauth, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0",
        auth_gateway_base
    ))
    .await?;
    let _ = ws_wait_for(&mut unauth, "hello").await?;
    let _ = ws_wait_for(&mut unauth, "replay").await?;
    unauth
        .send(Message::Text(
            json!({
                "type": "intent.batch",
                "intents": [{
                    "id": "i-unauth-0001",
                    "intent": "TODO_CREATE",
                    "payload": {"id":"todo-x"},
                    "meta": {"clock": 1}
                }]
            })
            .to_string(),
        ))
        .await?;
    let unauth_error = ws_wait_for(&mut unauth, "error").await?;
    assert_eq!(unauth_error["code"], "UNAUTHORIZED");

    // E channel snapshot
    ws.send(Message::Text(
        json!({ "type": "channel.subscribe", "channel": "global", "params": { "scope": "all" } })
            .to_string(),
    ))
    .await?;
    let sub_ack = ws_wait_for(&mut ws, "channel.ack").await?;
    assert_eq!(sub_ack["status"], "ok");
    assert_eq!(sub_ack["params"], json!({ "scope": "all" }));
    let snapshot = ws_wait_for(&mut ws, "channel.snapshot").await?;
    assert_eq!(snapshot["channel"], "global");
    assert_eq!(snapshot["params"], json!({ "scope": "all" }));

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
        })
        .to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "ack").await?;
    let invalidate = ws_wait_for(&mut ws, "channel.invalidate").await?;
    assert_eq!(invalidate["type"], "channel.invalidate");
    assert_eq!(invalidate["channel"], "global");
    assert_eq!(invalidate["params"], json!({ "scope": "all" }));

    // observability endpoint
    let gateway_metrics = reqwest::get(format!("http://{}/optimistic/metrics", gateway_base))
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(gateway_metrics["status"], "ok");
    assert!(
        gateway_metrics["totals"]["broadcasts"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        gateway_metrics["serverEvents"]["CHANNEL_SUBSCRIBE"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        gateway_metrics["serverEvents"]["INTENT_ACKED"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );

    // RBAC deny on protected channels
    let (rbac_gateway_base, rbac_gateway_handle) =
        spawn_gateway_with(backend_base.clone(), false, |config| {
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
            json!({ "type": "channel.subscribe", "channel": "admin-only", "params": {} })
                .to_string(),
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

#[tokio::test]
async fn jwt_auth_controls_protected_channels_and_island_invalidations() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let jwt_secret = "test-secret";
    let (gateway_base, gateway_handle) = spawn_gateway_with(backend_base, false, |config| {
        config.auth_jwt_secret = jwt_secret.to_string();
        config.protected_channels = vec!["ops.audit".to_string()];
        config.protected_channel_min_role = "staff".to_string();
    })
    .await?;

    let guest_url = format!(
        "ws://{}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0",
        gateway_base
    );
    let mut guest_ws = connect_with_cookie(guest_url, None).await?;
    let guest_hello = ws_wait_for(&mut guest_ws, "hello").await?;
    assert_eq!(guest_hello["authRole"], "guest");
    let _ = ws_wait_for(&mut guest_ws, "replay").await?;
    guest_ws
        .send(Message::Text(
            json!({
                "type": "channel.subscribe",
                "channel": "ops.audit",
                "params": { "scope": "secure" }
            })
            .to_string(),
        ))
        .await?;
    let guest_ack = ws_wait_for(&mut guest_ws, "channel.ack").await?;
    assert_eq!(guest_ack["status"], "error");
    assert_eq!(guest_ack["code"], "ACCESS_DENIED");
    assert_eq!(guest_ack["params"], json!({ "scope": "secure" }));
    let guest_close = ws_wait_for(&mut guest_ws, "channel.close").await?;
    assert_eq!(guest_close["code"], "ACCESS_DENIED");

    let staff_cookie = issue_session_cookie(jwt_secret, "staff-user", "staff")?;
    let staff_url = format!(
        "ws://{}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0",
        gateway_base
    );
    let mut staff_ws = connect_with_cookie(staff_url, Some(&staff_cookie)).await?;
    let staff_hello = ws_wait_for(&mut staff_ws, "hello").await?;
    assert_eq!(staff_hello["authRole"], "staff");
    let _ = ws_wait_for(&mut staff_ws, "replay").await?;
    staff_ws
        .send(Message::Text(
            json!({
                "type": "channel.subscribe",
                "channel": "ops.audit",
                "params": { "scope": "secure" }
            })
            .to_string(),
        ))
        .await?;
    let staff_ack = ws_wait_for(&mut staff_ws, "channel.ack").await?;
    assert_eq!(staff_ack["status"], "ok");
    assert_eq!(staff_ack["params"], json!({ "scope": "secure" }));
    let staff_snapshot = ws_wait_for(&mut staff_ws, "channel.snapshot").await?;
    assert_eq!(staff_snapshot["channel"], "ops.audit");
    assert_eq!(staff_snapshot["params"], json!({ "scope": "secure" }));

    let guest_sse_base = gateway_base.clone();
    let staff_sse_base = gateway_base.clone();
    let staff_cookie_for_sse = staff_cookie.clone();
    let guest_sse = tokio::spawn(async move {
        sse_wait_for_with_headers_timeout(
            &guest_sse_base,
            &["island.invalidate"],
            Some("?islands=ops.dashboard"),
            None,
            Duration::from_millis(700),
        )
        .await
    });
    let staff_sse = tokio::spawn(async move {
        sse_wait_for_with_headers_timeout(
            &staff_sse_base,
            &["island.invalidate"],
            Some("?islands=ops.dashboard"),
            Some(&staff_cookie_for_sse),
            Duration::from_secs(3),
        )
        .await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let response = reqwest::Client::new()
        .post(format!("http://{}/internal/backend/events", gateway_base))
        .json(&json!({
            "event": {
                "site": "default",
                "reason": "ops-refresh",
                "islandId": "ops.dashboard",
                "timestamp": now_millis(),
                "payload": { "status": "green" }
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    let staff_sse_event = staff_sse.await??;
    assert_eq!(staff_sse_event["type"], "island.invalidate");
    assert_eq!(staff_sse_event["data"]["islandId"], "ops.dashboard");
    assert_eq!(staff_sse_event["data"]["reason"], "ops-refresh");

    let guest_sse_result = guest_sse.await?;
    assert!(
        guest_sse_result.is_err(),
        "guest SSE unexpectedly received protected island event"
    );

    let staff_ws_event =
        ws_wait_for_with_timeout(&mut staff_ws, "island.invalidate", Duration::from_secs(2))
            .await?;
    assert_eq!(staff_ws_event["islandId"], "ops.dashboard");
    assert_eq!(staff_ws_event["reason"], "ops-refresh");

    let guest_island_event = ws_wait_for_with_timeout(
        &mut guest_ws,
        "island.invalidate",
        Duration::from_millis(700),
    )
    .await;
    assert!(
        guest_island_event.is_err(),
        "guest WS unexpectedly received protected island event"
    );

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn media_assets_enforce_cookie_ownership_and_backend_token() -> Result<()> {
    let (gateway_base, gateway_handle) = spawn_gateway_with(String::new(), false, |config| {
        config.backend_internal_token = "test-backend-token".to_string();
        config.media_max_upload_bytes = 1_024;
    })
    .await?;

    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(b"RIFFtest-wave".to_vec())
            .file_name("sample.wav")
            .mime_str("audio/wav")?,
    );
    let upload = client
        .post(format!("http://{}/media/assets", gateway_base))
        .multipart(form)
        .send()
        .await?;
    assert_eq!(upload.status(), reqwest::StatusCode::CREATED);
    let anon_cookie = extract_cookie(&upload, "ssma_anon").expect("anonymous cookie");
    let upload_json = upload.json::<Value>().await?;
    let asset_id = upload_json["asset"]["assetId"]
        .as_str()
        .expect("asset id")
        .to_string();
    assert_eq!(upload_json["asset"]["mediaType"], "audio");

    let unauthorized = client
        .get(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .send()
        .await?;
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let owned_metadata = client
        .get(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(owned_metadata.status(), reqwest::StatusCode::OK);

    let content = client
        .get(format!(
            "http://{}/media/assets/{}/content",
            gateway_base, asset_id
        ))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.bytes().await?.as_ref(), b"RIFFtest-wave");

    let internal_denied = client
        .get(format!(
            "http://{}/internal/assets/{}",
            gateway_base, asset_id
        ))
        .send()
        .await?;
    assert_eq!(internal_denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let internal_ok = client
        .get(format!(
            "http://{}/internal/assets/{}",
            gateway_base, asset_id
        ))
        .header("x-ssma-backend-token", "test-backend-token")
        .send()
        .await?;
    assert_eq!(internal_ok.status(), reqwest::StatusCode::OK);

    let internal_content = client
        .get(format!(
            "http://{}/internal/assets/{}/content",
            gateway_base, asset_id
        ))
        .header("x-ssma-backend-token", "test-backend-token")
        .send()
        .await?;
    assert_eq!(internal_content.status(), reqwest::StatusCode::OK);
    assert_eq!(internal_content.bytes().await?.as_ref(), b"RIFFtest-wave");

    let oversized_form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(vec![0_u8; 1_025])
            .file_name("too-big.wav")
            .mime_str("audio/wav")?,
    );
    let oversized = client
        .post(format!("http://{}/media/assets", gateway_base))
        .multipart(oversized_form)
        .send()
        .await?;
    assert!(
        oversized.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE
            || oversized.status() == reqwest::StatusCode::BAD_REQUEST
    );

    let deleted = client
        .delete(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(deleted.status(), reqwest::StatusCode::OK);

    let missing = client
        .get(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

    gateway_handle.abort();
    Ok(())
}

#[tokio::test]
async fn public_query_issues_cookie_and_forwards_actor_context() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle) = spawn_gateway(backend_base, false).await?;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("http://{}/query/echo-context", gateway_base))
        .header("user-agent", "cargo-test-query")
        .json(&json!({ "payload": { "message": "hello" } }))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let anon_cookie = extract_cookie(&response, "ssma_anon").expect("anonymous cookie");
    let json = response.json::<Value>().await?;

    assert_eq!(json["status"], "ok");
    assert_eq!(json["data"]["payload"]["message"], "hello");
    assert_eq!(json["data"]["context"]["site"], "default");
    assert_eq!(json["data"]["context"]["user"]["role"], "guest");
    assert_eq!(json["data"]["context"]["user"]["id"], Value::Null);
    assert!(json["data"]["context"]["actorKey"]
        .as_str()
        .unwrap_or_default()
        .starts_with("anon:"));

    let query_with_cookie = client
        .post(format!("http://{}/query/echo-context", gateway_base))
        .header("Cookie", &anon_cookie)
        .json(&json!({ "payload": { "message": "again" } }))
        .send()
        .await?;
    assert_eq!(query_with_cookie.status(), reqwest::StatusCode::OK);
    let with_cookie_json = query_with_cookie.json::<Value>().await?;
    assert_eq!(
        with_cookie_json["data"]["context"]["actorKey"],
        json["data"]["context"]["actorKey"]
    );

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn backend_created_assets_are_publicly_readable_by_same_actor_via_query() -> Result<()> {
    let backend_state = ToyBackendState::default();
    {
        *backend_state
            .backend_token
            .lock()
            .expect("backend token lock") = Some("test-backend-token".to_string());
    }
    let (backend_base, backend_handle) =
        spawn_toy_backend_with_state(backend_state.clone()).await?;
    let (gateway_base, gateway_handle) = spawn_gateway_with(backend_base, false, |config| {
        config.backend_internal_token = "test-backend-token".to_string();
    })
    .await?;
    {
        *backend_state
            .gateway_base
            .lock()
            .expect("gateway base lock") = Some(gateway_base.clone());
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/query/create-output-asset", gateway_base))
        .json(&json!({
            "payload": {
                "fileName": "speech.wav",
                "mimeType": "audio/wav",
                "mediaType": "audio",
                "content": "RIFFgenerated-audio"
            }
        }))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let anon_cookie = extract_cookie(&response, "ssma_anon").expect("anonymous cookie");
    let json = response.json::<Value>().await?;
    let asset_id = json["data"]["assetId"]
        .as_str()
        .expect("asset id")
        .to_string();
    assert_eq!(json["data"]["mediaType"], "audio");

    let metadata = client
        .get(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(metadata.status(), reqwest::StatusCode::OK);

    let content = client
        .get(format!(
            "http://{}/media/assets/{}/content",
            gateway_base, asset_id
        ))
        .header("Cookie", &anon_cookie)
        .send()
        .await?;
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.bytes().await?.as_ref(), b"RIFFgenerated-audio");

    let denied = client
        .get(format!("http://{}/media/assets/{}", gateway_base, asset_id))
        .send()
        .await?;
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let internal_deleted = client
        .delete(format!(
            "http://{}/internal/assets/{}",
            gateway_base, asset_id
        ))
        .header("x-ssma-backend-token", "test-backend-token")
        .send()
        .await?;
    assert_eq!(internal_deleted.status(), reqwest::StatusCode::OK);

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn rtc_signals_are_ephemeral_and_require_shared_actor_identity() -> Result<()> {
    let (gateway_base, gateway_handle) = spawn_gateway(String::new(), false).await?;
    let client = reqwest::Client::new();

    let create = client
        .post(format!("http://{}/rtc/sessions", gateway_base))
        .json(&json!({}))
        .send()
        .await?;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let anon_cookie = extract_cookie(&create, "ssma_anon").expect("anonymous cookie");
    let create_json = create.json::<Value>().await?;
    let session_id = create_json["session"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_string();
    let channel = create_json["session"]["channel"]
        .as_str()
        .expect("channel")
        .to_string();

    let ws_url = format!(
        "ws://{}/optimistic/ws?role=follower&site=default&subprotocol=1.0.0",
        gateway_base
    );
    let mut ws = connect_with_cookie(ws_url.clone(), Some(&anon_cookie)).await?;
    let _ = ws_wait_for(&mut ws, "hello").await?;
    let replay = ws_wait_for(&mut ws, "replay").await?;
    assert_eq!(
        replay["intents"]
            .as_array()
            .map(|rows| rows.len())
            .unwrap_or(0),
        0
    );

    ws.send(Message::Text(
        json!({ "type": "channel.subscribe", "channel": channel, "params": {} }).to_string(),
    ))
    .await?;
    let ack = ws_wait_for(&mut ws, "channel.ack").await?;
    assert_eq!(ack["status"], "ok");
    let snapshot = ws_wait_for(&mut ws, "channel.snapshot").await?;
    assert_eq!(
        snapshot["intents"]
            .as_array()
            .map(|rows| rows.len())
            .unwrap_or(0),
        0
    );

    let outsider = client
        .post(format!(
            "http://{}/rtc/sessions/{}/signals",
            gateway_base, session_id
        ))
        .json(&json!({
            "kind": "offer",
            "senderId": "peer-a",
            "payload": { "sdp": "v=0" }
        }))
        .send()
        .await?;
    assert_eq!(outsider.status(), reqwest::StatusCode::UNAUTHORIZED);

    let signal_response = client
        .post(format!(
            "http://{}/rtc/sessions/{}/signals",
            gateway_base, session_id
        ))
        .header("Cookie", &anon_cookie)
        .json(&json!({
            "kind": "offer",
            "senderId": "peer-a",
            "targetId": "peer-b",
            "payload": { "sdp": "v=0" }
        }))
        .send()
        .await?;
    assert_eq!(signal_response.status(), reqwest::StatusCode::OK);

    let invalidate = ws_wait_for(&mut ws, "channel.invalidate").await?;
    assert_eq!(invalidate["channel"], create_json["session"]["channel"]);
    assert_eq!(invalidate["intents"][0]["payload"]["kind"], "offer");

    let mut replay_ws = connect_with_cookie(ws_url, Some(&anon_cookie)).await?;
    let _ = ws_wait_for(&mut replay_ws, "hello").await?;
    let replay_after = ws_wait_for(&mut replay_ws, "replay").await?;
    assert_eq!(
        replay_after["intents"]
            .as_array()
            .map(|rows| rows.len())
            .unwrap_or(0),
        0,
        "rtc signals must stay out of durable replay"
    );
    replay_ws
        .send(Message::Text(
            json!({ "type": "channel.subscribe", "channel": create_json["session"]["channel"], "params": {} }).to_string(),
        ))
        .await?;
    let _ = ws_wait_for(&mut replay_ws, "channel.ack").await?;
    let replay_snapshot = ws_wait_for(&mut replay_ws, "channel.snapshot").await?;
    assert_eq!(
        replay_snapshot["intents"][0]["payload"]["targetId"],
        "peer-b"
    );

    gateway_handle.abort();
    Ok(())
}


#[tokio::test]
async fn sse_events_include_retry_field_matching_config() -> Result<()> {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await?;
    let backend_port = backend_listener.local_addr()?.port();
    let backend_base = format!("127.0.0.1:{}", backend_port);
    let toy_state = ToyBackendState::default();
    let backend_app = Router::new()
        .route("/apply", post(toy_apply))
        .route("/metrics", get(toy_metrics))
        .route("/query/:name", post(toy_query))
        .route("/subscribe", post(toy_subscribe))
        .route("/health", get(toy_health))
        .with_state(toy_state);
    tokio::spawn(async move {
        let _ = axum::serve(backend_listener, backend_app).await;
    });

    let custom_retry_ms: u64 = 500;
    let (gateway_base, gateway_handle) = spawn_gateway_with(
        format!("http://{}", backend_base),
        false,
        |config| {
            config.sse_retry_ms = custom_retry_ms;
        },
    )
    .await?;

    // Connect to SSE endpoint and read raw response
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://{}/optimistic/events?site=default",
            gateway_base
        ))
        .timeout(Duration::from_secs(3))
        .send()
        .await?;

    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(500), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(chunk.as_ref()));
                if buf.contains("retry:") && buf.contains("ready") {
                    break;
                }
            }
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            _ => continue,
        }
    }

    assert!(
        buf.contains(&format!("retry:{}", custom_retry_ms)),
        "SSE stream should contain retry:{} field, got: {}",
        custom_retry_ms,
        buf
    );

    gateway_handle.abort();
    Ok(())
}

#[tokio::test]
async fn ws_connection_closed_on_backpressure() -> Result<()> {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await?;
    let backend_port = backend_listener.local_addr()?.port();
    let backend_base = format!("127.0.0.1:{}", backend_port);
    let toy_state = ToyBackendState::default();
    let backend_app = Router::new()
        .route("/apply", post(toy_apply))
        .route("/metrics", get(toy_metrics))
        .route("/query/:name", post(toy_query))
        .route("/subscribe", post(toy_subscribe))
        .route("/health", get(toy_health))
        .with_state(toy_state);
    tokio::spawn(async move {
        let _ = axum::serve(backend_listener, backend_app).await;
    });

    // Set very low backpressure limit: 1024 bytes => 1 pending message max
    let (gateway_base, gateway_handle) = spawn_gateway_with(
        format!("http://{}", backend_base),
        false,
        |config| {
            config.ws_max_buffered_bytes = 1024;
        },
    )
    .await?;

    // Connect WS client and subscribe to a channel
    let mut ws = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0",
        gateway_base
    ))
    .await?
    .0;
    let _hello = ws_wait_for(&mut ws, "hello").await?;
    let _replay = ws_wait_for(&mut ws, "replay").await?;

    ws.send(Message::Text(
        json!({ "type": "channel.subscribe", "channel": "global", "params": { "scope": "all" } })
            .to_string(),
    ))
    .await?;
    let _snapshot = ws_wait_for(&mut ws, "channel.snapshot").await?;

    // Now flood the broadcast channel by sending many intent batches
    // Each triggers a broadcast event. With max_pending=1, the subscriber
    // should hit backpressure quickly.
    for i in 0..20 {
        ws.send(Message::Text(
            json!({
                "type": "intent.batch",
                "intents": [{
                    "id": format!("bp-{}", i),
                    "intent": "TODO_CREATE",
                    "payload": {"id": format!("todo-bp-{}", i)},
                    "meta": {"clock": 1000 + i, "channels": ["global"]}
                }]
            })
            .to_string(),
        ))
        .await?;
    }

    // Wait for either BACKPRESSURE_CLOSE or connection close
    let got_backpressure = timeout(Duration::from_secs(5), async {
        loop {
            match timeout(Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let msg: Value = serde_json::from_str(&text).unwrap_or_default();
                    if msg["code"] == "BACKPRESSURE_CLOSE" {
                        return true;
                    }
                }
                Ok(Some(Ok(Message::Close(_)))) => return true,
                Ok(None) => return true, // connection closed
                _ => continue,
            }
        }
    })
    .await;

    assert!(
        got_backpressure.is_ok(),
        "WS connection should be closed due to backpressure"
    );

    gateway_handle.abort();
    Ok(())
}

fn now_millis() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

// --- Graceful shutdown helpers and tests ---

/// Spawns a gateway using `serve_with_shutdown` with a oneshot channel as the
/// shutdown signal. Returns `(gateway_base, server_join_handle, shutdown_tx)`.
/// Calling `shutdown_tx.send(())` triggers graceful shutdown.
async fn spawn_gateway_with_shutdown(
    backend_url: &str,
    require_auth: bool,
    configure: impl FnOnce(&mut ssma_rust::runtime::Config),
) -> Result<(
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
    std::path::PathBuf,
)> {
    let mut config = ssma_rust::runtime::Config::from_env();
    config.host = "127.0.0.1".to_string();
    config.port = 0;
    config.backend_url = backend_url.to_string();
    config.require_auth_for_writes = require_auth;
    config.intent_store_path = std::env::temp_dir().join(format!(
        "ssma-rust-e2e-shutdown-{}.json",
        uuid::Uuid::new_v4()
    ));
    configure(&mut config);

    let intent_store_path = config.intent_store_path.clone();
    let state = ssma_rust::gateway::build_state(config);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        let signal = async {
            let _ = shutdown_rx.await;
            Ok::<(), std::io::Error>(())
        };
        let _ = ssma_rust::gateway::serve_with_shutdown(listener, state, signal).await;
    });

    // Wait for the server to start accepting connections
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok((format!("127.0.0.1:{}", addr.port()), handle, shutdown_tx, intent_store_path))
}

#[tokio::test]
async fn shutdown_signal_triggers_graceful_shutdown() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle, shutdown_tx, _) =
        spawn_gateway_with_shutdown(&backend_base, false, |_| {}).await?;

    // Verify server is up
    let health = reqwest::get(format!("http://{}/health", gateway_base))
        .await?
        .json::<Value>()
        .await?;
    assert_eq!(health["status"], "ok");

    // Trigger shutdown
    shutdown_tx.send(()).expect("send shutdown signal");

    // Server should complete within a reasonable time
    let result = timeout(Duration::from_secs(5), gateway_handle).await;
    assert!(result.is_ok(), "server should shut down within 5s");

    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn active_ws_connections_receive_close_frame_on_shutdown() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle, shutdown_tx, _) =
        spawn_gateway_with_shutdown(&backend_base, false, |_| {}).await?;

    // Connect a WS client
    let (mut ws, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0",
        gateway_base
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "hello").await?;
    let _ = ws_wait_for(&mut ws, "replay").await?;

    // Trigger shutdown
    shutdown_tx.send(()).expect("send shutdown signal");

    // WS client should receive a close frame
    let got_close = timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(Message::Text(text))) => {
                    // Might receive a few buffered messages before close
                    let msg: Value = serde_json::from_str(&text).unwrap_or_default();
                    if msg["type"] == "server.shutdown" {
                        continue;
                    }
                }
                Some(Err(_)) => return true, // connection closed with error
                None => return true,         // stream ended
                _ => continue,
            }
        }
    })
    .await;

    assert!(got_close.is_ok(), "WS connection should close within 5s of shutdown signal");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn intent_store_written_on_shutdown() -> Result<()> {
    let (backend_base, backend_handle) = spawn_toy_backend().await?;
    let (gateway_base, gateway_handle, shutdown_tx, intent_store_path) =
        spawn_gateway_with_shutdown(&backend_base, false, |_| {}).await?;

    // Send an intent via WS
    let (mut ws, _) = connect_async(format!(
        "ws://{}/optimistic/ws?role=leader&site=default&subprotocol=1.0.0",
        gateway_base
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "hello").await?;
    let _ = ws_wait_for(&mut ws, "replay").await?;

    ws.send(Message::Text(
        json!({
            "type": "intent.batch",
            "intents": [{
                "id": "i-shutdown-test",
                "intent": "TODO_CREATE",
                "payload": {"id":"todo-shutdown"},
                "meta": {"clock": now_millis(), "channels": ["global"]}
            }]
        })
        .to_string(),
    ))
    .await?;
    let _ = ws_wait_for(&mut ws, "ack").await?;

    // Trigger shutdown
    shutdown_tx.send(()).expect("send shutdown signal");
    let _ = timeout(Duration::from_secs(5), gateway_handle).await;

    // Verify intent store file was written
    assert!(intent_store_path.exists(), "intent store file should exist after shutdown");
    let contents = std::fs::read_to_string(&intent_store_path)?;
    let persisted: Value = serde_json::from_str(&contents)?;
    assert_eq!(persisted["version"], 1);
    let entries = persisted["entries"].as_array().expect("entries array");
    assert!(
        entries.iter().any(|e| e["id"] == "i-shutdown-test"),
        "intent store should contain the test intent after shutdown"
    );

    // Cleanup
    let _ = std::fs::remove_file(&intent_store_path);
    backend_handle.abort();
    Ok(())
}
