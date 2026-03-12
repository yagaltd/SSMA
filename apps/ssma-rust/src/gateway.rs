use crate::backend::{BackendContext, BackendHttpClient, BackendUser};
use crate::protocol;
use crate::runtime::{now_millis, now_secs, Config, IntentRecord, IntentStore};
use async_stream::stream;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub store: IntentStore,
    pub backend: BackendHttpClient,
    pub events: broadcast::Sender<Value>,
    assets: Arc<Mutex<HashMap<String, AssetRecord>>>,
    rtc_sessions: Arc<Mutex<HashMap<String, RtcSessionRecord>>>,
    channel_limits: Arc<Mutex<HashMap<String, RateBucket>>>,
    global_limits: Arc<Mutex<HashMap<String, RateBucket>>>,
    channel_registry: Arc<Mutex<HashMap<String, ConnectionChannels>>>,
    metrics: Arc<MetricsState>,
}

#[derive(Debug, Clone)]
struct AssetRecord {
    asset_id: String,
    site: String,
    owner_key: String,
    media_type: String,
    mime_type: String,
    file_name: Option<String>,
    size_bytes: u64,
    path: PathBuf,
    created_at_secs: u64,
    expires_at_secs: u64,
}

#[derive(Debug, Clone)]
struct RtcSignalRecord {
    seq: u64,
    kind: String,
    sender_id: String,
    target_id: Option<String>,
    payload: Value,
    created_at_ms: i64,
}

#[derive(Debug, Clone)]
struct RtcSessionRecord {
    session_id: String,
    site: String,
    owner_key: String,
    participants: Vec<String>,
    signals: Vec<RtcSignalRecord>,
    next_seq: u64,
    expires_at_secs: u64,
}

#[derive(Debug, Clone)]
struct ActorIdentity {
    actor_key: String,
    actor_id: String,
    role: String,
    user_id: Option<String>,
    set_cookie: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetMetadata {
    asset_id: String,
    site: String,
    media_type: String,
    mime_type: String,
    file_name: Option<String>,
    size_bytes: u64,
    created_at: u64,
    expires_at: u64,
}

#[derive(Debug, Clone)]
struct RateBucket {
    count: u32,
    expires_at_ms: i64,
}

#[derive(Debug, Default)]
struct MetricsState {
    ws_active: AtomicU64,
    sse_active: AtomicU64,
    ws_total: AtomicU64,
    sse_total: AtomicU64,
    broadcast_count: AtomicU64,
    rate_limit_hits: AtomicU64,
    sse_client_dropped: AtomicU64,
    ws_unauthorized_filtered: AtomicU64,
    sse_unauthorized_filtered: AtomicU64,
    server_events: Mutex<HashMap<String, u64>>,
}

#[derive(Debug, Clone)]
struct ChannelSubscription {
    channel: String,
    params: Value,
}

#[derive(Debug, Clone, Default)]
struct ConnectionChannels {
    site: String,
    subscriptions: HashMap<String, ChannelSubscription>,
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    role: Option<String>,
    site: Option<String>,
    subprotocol: Option<String>,
    cursor: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SseQuery {
    site: Option<String>,
    cursor: Option<u64>,
    islands: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BackendEventsPayload {
    events: Option<Vec<Value>>,
    event: Option<Value>,
}

#[derive(Debug, Clone)]
struct ConnectionContext {
    transport_role: String,
    auth_role: String,
    site: String,
    connection_id: String,
    actor_key: Option<String>,
    user_id: Option<String>,
    ip: String,
    user_agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthClaims {
    sub: String,
    role: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedUser {
    id: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRtcSessionRequest {
    participants: Option<Vec<String>>,
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostRtcSignalRequest {
    kind: String,
    sender_id: String,
    target_id: Option<String>,
    payload: Value,
}

pub fn build_state(config: Config) -> Arc<AppState> {
    let _ = fs::create_dir_all(&config.media_storage_root);
    if let Ok(entries) = fs::read_dir(&config.media_storage_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }
    let store = IntentStore::new(config.intent_store_path.clone(), config.replay_window_ms);
    let backend = BackendHttpClient::new(config.backend_url.clone());
    let (events, _) = broadcast::channel(1024);
    Arc::new(AppState {
        config,
        store,
        backend,
        events,
        assets: Arc::new(Mutex::new(HashMap::new())),
        rtc_sessions: Arc::new(Mutex::new(HashMap::new())),
        channel_limits: Arc::new(Mutex::new(HashMap::new())),
        global_limits: Arc::new(Mutex::new(HashMap::new())),
        channel_registry: Arc::new(Mutex::new(HashMap::new())),
        metrics: Arc::new(MetricsState::default()),
    })
}

pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/media/assets", post(upload_media))
        .route(
            "/media/assets/:asset_id",
            get(get_asset_metadata).delete(delete_asset),
        )
        .route("/media/assets/:asset_id/content", get(get_asset_content))
        .route("/rtc/sessions", post(create_rtc_session))
        .route("/rtc/sessions/:session_id/signals", post(post_rtc_signal))
        .route("/optimistic/metrics", get(metrics))
        .route("/optimistic/ws", get(ws_upgrade))
        .route("/optimistic/events", get(sse_events))
        .route("/internal/backend/events", post(backend_events_ingest))
        .route(
            "/internal/assets/:asset_id",
            get(get_internal_asset_metadata),
        )
        .route(
            "/internal/assets/:asset_id/content",
            get(get_internal_asset_content),
        )
        .layer(DefaultBodyLimit::max(
            state.config.media_max_upload_bytes as usize,
        ))
        .with_state(state)
}

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let state = build_state(config.clone());
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await?;
    tracing::info!(addr = %listener.local_addr()?, "ssma-rust listening");
    serve(listener, state).await
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    axum::serve(listener, app(state)).await?;
    Ok(())
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "ssma-rust",
        "subprotocol": state.config.subprotocol,
        "cursor": state.store.latest_cursor(),
    }))
}

async fn metrics(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "ssma-rust",
        "active": {
            "ws": state.metrics.ws_active.load(Ordering::Relaxed),
            "sse": state.metrics.sse_active.load(Ordering::Relaxed),
        },
        "totals": {
            "wsConnections": state.metrics.ws_total.load(Ordering::Relaxed),
            "sseConnections": state.metrics.sse_total.load(Ordering::Relaxed),
            "broadcasts": state.metrics.broadcast_count.load(Ordering::Relaxed),
            "rateLimitHits": state.metrics.rate_limit_hits.load(Ordering::Relaxed),
            "sseClientDropped": state.metrics.sse_client_dropped.load(Ordering::Relaxed),
            "wsUnauthorizedFiltered": state.metrics.ws_unauthorized_filtered.load(Ordering::Relaxed),
            "sseUnauthorizedFiltered": state.metrics.sse_unauthorized_filtered.load(Ordering::Relaxed),
        },
        "store": {
            "cursor": state.store.latest_cursor(),
            "replayDepth": state.store.total_entries(),
        },
        "serverEvents": state.metrics.server_events.lock().expect("server events lock").clone(),
    }))
}

type ApiError = (StatusCode, Json<Value>);
type ApiResult<T> = Result<T, ApiError>;

async fn upload_media(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);

    let mut selected: Option<(String, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let mime = field
            .content_type()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let file_name = field.file_name().map(|value| value.to_string());
        let bytes = field.bytes().await.map_err(multipart_error)?;
        if bytes.is_empty() {
            continue;
        }
        selected = Some((mime, file_name, bytes.to_vec()));
        break;
    }

    let Some((mime_type, file_name, bytes)) = selected else {
        return Err(api_error(StatusCode::BAD_REQUEST, "MISSING_MEDIA"));
    };

    let media_type = media_type_from_mime(&mime_type)
        .ok_or_else(|| api_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE"))?;
    if bytes.len() as u64 > state.config.media_max_upload_bytes {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
        ));
    }

    let asset_id = Uuid::new_v4().to_string();
    let path = state
        .config
        .media_storage_root
        .join(format!("{}.bin", &asset_id));
    fs::write(&path, &bytes)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "ASSET_WRITE_FAILED"))?;

    let now = now_secs();
    let record = AssetRecord {
        asset_id: asset_id.clone(),
        site: site.clone(),
        owner_key: actor.actor_key.clone(),
        media_type: media_type.to_string(),
        mime_type,
        file_name,
        size_bytes: bytes.len() as u64,
        path,
        created_at_secs: now,
        expires_at_secs: now + state.config.media_ttl_secs,
    };
    let metadata = asset_metadata(&record);
    state
        .assets
        .lock()
        .expect("assets lock")
        .insert(asset_id.clone(), record);

    emit_server_event(
        &state,
        "MEDIA_ASSET_CREATED",
        json!({"assetId": asset_id, "site": site, "mediaType": media_type}),
    );

    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }

    Ok((
        StatusCode::CREATED,
        response_headers,
        Json(json!({ "status": "ok", "asset": metadata })),
    ))
}

async fn get_asset_metadata(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    Ok(Json(
        json!({ "status": "ok", "asset": asset_metadata(&record) }),
    ))
}

async fn get_asset_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    let bytes = fs::read(&record.path)
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "ASSET_CONTENT_MISSING"))?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&record.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, response_headers, bytes))
}

async fn delete_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    {
        let mut assets = state.assets.lock().expect("assets lock");
        assets.remove(&record.asset_id);
    }
    let _ = fs::remove_file(&record.path);
    emit_server_event(
        &state,
        "MEDIA_ASSET_DELETED",
        json!({"assetId": asset_id, "site": site}),
    );
    Ok(Json(json!({ "status": "ok", "deleted": true })))
}

async fn get_internal_asset_metadata(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;
    let record = internal_asset(&state, &asset_id)?;
    Ok(Json(
        json!({ "status": "ok", "asset": asset_metadata(&record) }),
    ))
}

async fn get_internal_asset_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;
    let record = internal_asset(&state, &asset_id)?;
    let bytes = fs::read(&record.path)
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "ASSET_CONTENT_MISSING"))?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&record.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, response_headers, bytes))
}

async fn create_rtc_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateRtcSessionRequest>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let session_id = Uuid::new_v4().to_string();
    let mut participants = vec![actor.actor_key.clone()];
    for participant in body.participants.unwrap_or_default() {
        if !participants.iter().any(|value| value == &participant) {
            participants.push(participant);
        }
    }
    let now = now_secs();
    let ttl = body.ttl_secs.unwrap_or(state.config.media_ttl_secs);
    let record = RtcSessionRecord {
        session_id: session_id.clone(),
        site: site.clone(),
        owner_key: actor.actor_key.clone(),
        participants,
        signals: Vec::new(),
        next_seq: 0,
        expires_at_secs: now + ttl,
    };
    state
        .rtc_sessions
        .lock()
        .expect("rtc sessions lock")
        .insert(session_id.clone(), record);
    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }
    Ok((
        StatusCode::CREATED,
        response_headers,
        Json(json!({
            "status": "ok",
            "session": {
                "sessionId": session_id.clone(),
                "channel": rtc_channel_name(&session_id),
                "site": site,
                "owner": actor.actor_id,
                "createdAt": now,
                "expiresAt": now + ttl,
            }
        })),
    ))
}

async fn post_rtc_signal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<PostRtcSignalRequest>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let channel = rtc_channel_name(&session_id);

    let signal = {
        let mut sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
        let Some(session) = sessions.get_mut(&session_id) else {
            return Err(api_error(StatusCode::NOT_FOUND, "RTC_SESSION_NOT_FOUND"));
        };
        if session.site != site {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "RTC_SESSION_SITE_MISMATCH",
            ));
        }
        if session.owner_key != actor.actor_key
            && !session
                .participants
                .iter()
                .any(|participant| participant == &actor.actor_key)
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "RTC_SESSION_ACCESS_DENIED",
            ));
        }
        session.next_seq += 1;
        let signal = RtcSignalRecord {
            seq: session.next_seq,
            kind: body.kind.clone(),
            sender_id: body.sender_id.clone(),
            target_id: body.target_id.clone(),
            payload: body.payload.clone(),
            created_at_ms: now_millis(),
        };
        session.signals.push(signal.clone());
        if session.signals.len() > 64 {
            let overflow = session.signals.len() - 64;
            session.signals.drain(0..overflow);
        }
        signal
    };

    broadcast_app_event(
        &state,
        json!({
            "type": "invalidate",
            "reason": "rtc-signal",
            "site": site,
            "cursor": signal.seq,
            "intents": [rtc_signal_event_intent(&session_id, &signal, &channel)],
        }),
    );

    Ok(Json(json!({
        "status": "ok",
        "sessionId": session_id,
        "seq": signal.seq,
    })))
}

async fn backend_events_ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BackendEventsPayload>,
) -> impl IntoResponse {
    if !state.config.backend_internal_token.is_empty() {
        let token = headers
            .get("x-ssma-backend-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if token != state.config.backend_internal_token {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "UNAUTHORIZED_BACKEND_EVENT_SOURCE" })),
            );
        }
    }

    let mut processed = 0usize;
    let mut events = body.events.unwrap_or_default();
    if let Some(event) = body.event {
        events.push(event);
    }
    for event in events {
        processed += 1;
        publish_backend_event(&state, &event);
    }

    (
        StatusCode::ACCEPTED,
        Json(json!({ "status": "accepted", "processed": processed })),
    )
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, query, headers, state))
}

async fn ws_session(socket: WebSocket, query: WsQuery, headers: HeaderMap, state: Arc<AppState>) {
    state.metrics.ws_total.fetch_add(1, Ordering::Relaxed);
    state.metrics.ws_active.fetch_add(1, Ordering::Relaxed);

    let transport_role = query.role.unwrap_or_else(|| "follower".to_string());
    let site = query.site.unwrap_or_else(|| "default".to_string());
    let connection_id = Uuid::new_v4().to_string();
    let actor = resolve_actor_from_headers(&headers, &state.config, false);
    let ip = connection_ip_from_headers(&headers);
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let user_id = actor.as_ref().and_then(|resolved| resolved.user_id.clone());
    let auth_role = actor
        .as_ref()
        .map(|resolved| resolved.role.clone())
        .unwrap_or_else(|| "guest".to_string());
    let context = ConnectionContext {
        transport_role: transport_role.clone(),
        auth_role: auth_role.clone(),
        site: site.clone(),
        connection_id: connection_id.clone(),
        actor_key: actor.as_ref().map(|value| value.actor_key.clone()),
        user_id,
        ip,
        user_agent,
    };

    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.events.subscribe();

    let client_subprotocol = query
        .subprotocol
        .clone()
        .unwrap_or_else(|| state.config.subprotocol.clone());
    if !subprotocol_major_match(&state.config.subprotocol, &client_subprotocol) {
        let _ = sender
            .send(Message::Text(
                json!({
                    "type": "error",
                    "code": "SUBPROTOCOL_MISMATCH",
                    "expected": state.config.subprotocol
                })
                .to_string(),
            ))
            .await;
        let _ = sender.send(Message::Close(None)).await;
        teardown_connection_state(&state, &connection_id);
        return;
    }

    let hello = json!({
        "type": "hello",
        "role": transport_role,
        "authRole": auth_role,
        "subprotocol": state.config.subprotocol,
        "connectionId": connection_id,
        "serverTime": now_millis(),
    });
    let _ = sender.send(Message::Text(hello.to_string())).await;

    let cursor = query.cursor.unwrap_or(0);
    let replay = state.store.entries_after(cursor, 500);
    let replay_cursor = replay.last().map(|entry| entry.log_seq).unwrap_or(cursor);
    let _ = sender
        .send(Message::Text(
            json!({ "type": "replay", "intents": replay, "cursor": replay_cursor }).to_string(),
        ))
        .await;

    loop {
        tokio::select! {
            maybe_msg = receiver.next() => {
                let Some(Ok(message)) = maybe_msg else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };

                let global_key = format!("ws:{}", context.connection_id);
                let globally_allowed = consume_global_rate_limit(
                    &state,
                    global_key,
                    state.config.global_rate_max,
                    state.config.global_rate_window_ms,
                );
                if !globally_allowed {
                    state.metrics.rate_limit_hits.fetch_add(1, Ordering::Relaxed);
                    let _ = sender
                        .send(Message::Text(
                            json!({ "type": "error", "code": "RATE_LIMITED", "retryAfterMs": state.config.global_rate_window_ms }).to_string(),
                        ))
                        .await;
                    continue;
                }

                let payload = match serde_json::from_str::<Value>(&text) {
                    Ok(v) => v,
                    Err(_) => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "error", "code": "INVALID_JSON" }).to_string()))
                            .await;
                        continue;
                    }
                };

                if let Err(details) = protocol::validate_inbound(&payload) {
                    let _ = sender
                        .send(Message::Text(
                            json!({ "type": "error", "code": "INVALID_CONTRACT", "details": details }).to_string(),
                        ))
                        .await;
                    continue;
                }

                let msg_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                match msg_type {
                    "ping" => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "pong", "ts": now_millis() }).to_string()))
                            .await;
                    }
                    "intent.batch" => {
                        handle_intent_batch(&mut sender, &state, &context, payload).await;
                    }
                    "channel.subscribe" => {
                        handle_channel_subscribe(&mut sender, &state, &context, payload).await;
                    }
                    "channel.unsubscribe" => {
                        handle_channel_unsubscribe(&mut sender, &state, &context, payload).await;
                    }
                    "channel.resync" => {
                        handle_channel_resync(&mut sender, &state, &context, payload).await;
                    }
                    "channel.command" => {
                        handle_channel_command(&mut sender, &state, &context, payload).await;
                    }
                    _ => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "error", "code": "UNKNOWN_TYPE" }).to_string()))
                            .await;
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(event) => {
                        for frame in build_frames_for_connection(&state, &context, &event) {
                            let _ = sender.send(Message::Text(frame.to_string())).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    teardown_connection_state(&state, &connection_id);
}

async fn handle_intent_batch(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    if context.transport_role != "leader" {
        let _ = sender
            .send(Message::Text(
                json!({ "type": "error", "code": "NOT_LEADER" }).to_string(),
            ))
            .await;
        return;
    }
    if state.config.require_auth_for_writes && context.user_id.is_none() {
        let _ = sender
            .send(Message::Text(
                json!({ "type": "error", "code": "UNAUTHORIZED" }).to_string(),
            ))
            .await;
        emit_server_event(
            state,
            "INTENT_REJECTED",
            json!({"reason":"UNAUTHORIZED", "site": context.site}),
        );
        return;
    }

    let intents = payload
        .get("intents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let now = now_millis();
    let records = intents
        .iter()
        .map(|intent| IntentRecord {
            id: intent
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            intent: intent
                .get("intent")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string(),
            payload: intent.get("payload").cloned().unwrap_or(Value::Null),
            meta: intent.get("meta").cloned().unwrap_or_else(|| json!({})),
            inserted_at: now,
            log_seq: 0,
            site: context.site.clone(),
            status: "acked".to_string(),
            connection_id: Some(context.connection_id.clone()),
            backend: None,
        })
        .collect::<Vec<_>>();

    let outcome = state.store.append_batch(records);
    for row in &outcome.all {
        emit_server_event(
            state,
            "INTENT_ACCEPTED",
            json!({"id": row.id, "site": row.site, "logSeq": row.log_seq}),
        );
    }

    let mut status_by_id = HashMap::<String, String>::new();
    for replayed in &outcome.replayed {
        status_by_id.insert(replayed.id.clone(), replayed.status.clone());
    }
    for fresh in &outcome.fresh {
        status_by_id.insert(fresh.id.clone(), fresh.status.clone());
    }

    if !outcome.fresh.is_empty() {
        emit_server_event(
            state,
            "INTENT_FORWARDED",
            json!({"site": context.site, "count": outcome.fresh.len()}),
        );
        let backend_ctx = BackendContext {
            site: context.site.clone(),
            connection_id: Some(context.connection_id.clone()),
            ip: Some(context.ip.clone()),
            user_agent: context.user_agent.clone(),
            user: Some(BackendUser {
                id: context.user_id.clone(),
                role: context.auth_role.clone(),
            }),
        };
        let fresh_payload = outcome
            .fresh
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "intent": entry.intent,
                    "payload": entry.payload,
                    "meta": entry.meta,
                    "logSeq": entry.log_seq,
                    "insertedAt": entry.inserted_at,
                })
            })
            .collect::<Vec<_>>();

        match state
            .backend
            .apply_intents(fresh_payload, &backend_ctx)
            .await
        {
            Ok(result) => {
                if let Some(results) = result.get("results").and_then(|v| v.as_array()) {
                    for row in results {
                        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let status = normalize_status(row.get("status").and_then(|v| v.as_str()));
                        status_by_id.insert(id.to_string(), status.to_string());
                        state.store.update_status(
                            id,
                            &context.site,
                            status,
                            row.get("code").cloned(),
                        );
                        if let Some(events) = row.get("events").and_then(|v| v.as_array()) {
                            for event in events {
                                publish_backend_event(state, event);
                            }
                        }
                    }
                }
                if let Some(events) = result.get("events").and_then(|v| v.as_array()) {
                    for event in events {
                        publish_backend_event(state, event);
                    }
                }
            }
            Err(error) => {
                for row in &outcome.fresh {
                    status_by_id.insert(row.id.clone(), "failed".to_string());
                    state.store.update_status(
                        &row.id,
                        &row.site,
                        "failed",
                        Some(json!({ "message": error.to_string() })),
                    );
                }
            }
        }
    }

    let ack_intents = intents
        .iter()
        .map(|intent| {
            let id = intent.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            let persisted = state.store.get(id, &context.site);
            let status = status_by_id.get(id).cloned().unwrap_or_else(|| "acked".to_string());
            if status == "acked" {
                emit_server_event(state, "INTENT_ACKED", json!({"id": id, "site": context.site}));
            } else {
                emit_server_event(state, "INTENT_REJECTED", json!({"id": id, "site": context.site, "status": status}));
            }
            json!({
                "id": id,
                "status": status,
                "serverTimestamp": persisted.as_ref().map(|e| e.inserted_at).unwrap_or_else(now_millis),
                "site": context.site,
                "logSeq": persisted.as_ref().map(|e| e.log_seq).unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();

    let _ = sender
        .send(Message::Text(
            json!({ "type": "ack", "intents": ack_intents }).to_string(),
        ))
        .await;

    let invalidate_intents = outcome
        .all
        .iter()
        .filter(|entry| {
            status_by_id
                .get(&entry.id)
                .map(|s| s == "acked")
                .unwrap_or(true)
        })
        .map(|entry| {
            json!({
                "id": entry.id,
                "intent": entry.intent,
                "payload": entry.payload,
                "meta": entry.meta,
                "insertedAt": entry.inserted_at,
                "logSeq": entry.log_seq,
            })
        })
        .collect::<Vec<_>>();

    if !invalidate_intents.is_empty() {
        broadcast_app_event(
            state,
            json!({
                "type": "invalidate",
                "reason": "intent-flush",
                "site": context.site,
                "cursor": state.store.latest_cursor(),
                "intents": invalidate_intents,
            }),
        );
    }
}

async fn handle_channel_subscribe(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));

    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.ack", "status": "error", "channel": channel, "params": params.clone(), "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params.clone(), "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }

    let key = format!("{}:{}:{}", context.site, context.connection_id, channel);
    let rate_allowed = consume_channel_rate_limit(
        state,
        key,
        state.config.channel_subscribe_max,
        state.config.channel_subscribe_window_ms,
    );
    if !rate_allowed {
        state
            .metrics
            .rate_limit_hits
            .fetch_add(1, Ordering::Relaxed);
        let _ = sender
            .send(Message::Text(
                json!({
                    "type": "channel.ack",
                    "status": "error",
                    "channel": channel,
                    "code": "RATE_LIMITED",
                    "retryAfterMs": state.config.channel_subscribe_window_ms,
                })
                .to_string(),
            ))
            .await;
        return;
    }

    register_channel_subscription(
        state,
        &context.connection_id,
        &context.site,
        channel,
        params.clone(),
    );

    emit_server_event(
        state,
        "CHANNEL_SUBSCRIBE",
        json!({"channel": channel, "site": context.site, "connectionId": context.connection_id}),
    );

    let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.ack", "status": "ok", "channel": channel, "params": params.clone() }).to_string(),
            ))
            .await;

    let mut snapshot_cursor = state.store.latest_cursor();
    let intents =
        if let Some((rtc_intents, rtc_cursor)) = rtc_snapshot_for_channel(state, channel, 0) {
            snapshot_cursor = rtc_cursor;
            rtc_intents
        } else if state.backend.is_configured() {
            let backend_ctx = BackendContext {
                site: context.site.clone(),
                connection_id: Some(context.connection_id.clone()),
                ip: Some(context.ip.clone()),
                user_agent: context.user_agent.clone(),
                user: Some(BackendUser {
                    id: context.user_id.clone(),
                    role: context.auth_role.clone(),
                }),
            };
            match state
                .backend
                .subscribe(channel, params.clone(), &backend_ctx)
                .await
            {
                Ok(response) if response.get("status").and_then(|v| v.as_str()) == Some("ok") => {
                    snapshot_cursor = response
                        .get("cursor")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(snapshot_cursor);
                    response
                        .get("snapshot")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                }
                _ => store_entries_for_channel_after(state, channel, 0, 200)
                    .into_iter()
                    .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                    .collect(),
            }
        } else {
            store_entries_for_channel_after(state, channel, 0, 200)
                .into_iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                .collect()
        };

    let _ = sender
        .send(Message::Text(
            json!({
                "type": "channel.snapshot",
                "channel": channel,
                "params": params.clone(),
                "intents": intents,
                "cursor": snapshot_cursor,
            })
            .to_string(),
        ))
        .await;
}

async fn handle_channel_unsubscribe(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    unregister_channel_subscription(state, &context.connection_id, channel, &params);

    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.unsubscribed", "status": "ok", "channel": channel })
                .to_string(),
        ))
        .await;
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.close", "status": "ok", "channel": channel, "params": params, "reason": "client-unsubscribe" }).to_string(),
        ))
        .await;
}

async fn handle_channel_resync(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params, "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }
    let cursor = payload.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0);
    let (intents, next) =
        if let Some((rtc_intents, rtc_cursor)) = rtc_snapshot_for_channel(state, channel, cursor) {
            (rtc_intents, rtc_cursor)
        } else {
            let intents = store_entries_for_channel_after(state, channel, cursor, 200);
            let next = intents.last().map(|entry| entry.log_seq).unwrap_or(cursor);
            (
                intents
                    .into_iter()
                    .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                    .collect::<Vec<_>>(),
                next,
            )
        };
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.replay", "status": "ok", "channel": channel, "params": params, "cursor": next, "intents": intents }).to_string(),
        ))
        .await;
}

async fn handle_channel_command(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params, "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.command", "status": "ok", "channel": channel, "params": params, "command": command }).to_string(),
        ))
        .await;
}

async fn sse_events(
    Query(query): Query<SseQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let site = query.site.clone().unwrap_or_else(|| "default".to_string());
    let replay_cursor = query.cursor.unwrap_or(0);
    let islands = parse_requested_islands(query.islands.as_deref());
    let user = resolve_user_from_headers(&headers, &state.config);
    let auth_role = user
        .as_ref()
        .map(|resolved| resolved.role.clone())
        .unwrap_or_else(|| "guest".to_string());
    let replay = state.store.entries_after(replay_cursor, 500);
    let replay_cursor = replay
        .last()
        .map(|entry| entry.log_seq)
        .unwrap_or(replay_cursor);

    state.metrics.sse_total.fetch_add(1, Ordering::Relaxed);
    state.metrics.sse_active.fetch_add(1, Ordering::Relaxed);

    let mut rx = state.events.subscribe();
    let state_for_stream = state.clone();
    let stream = stream! {
        yield Ok(Event::default().event("ready").data(json!({ "service": "ssma-rust" }).to_string()));
        yield Ok(Event::default().event("replay").data(json!({ "intents": replay, "cursor": replay_cursor }).to_string()));

        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_site = event.get("site").and_then(|v| v.as_str()).unwrap_or("default");
                    if event_site != site {
                        continue;
                    }
                    if !is_sse_event_authorized(&state_for_stream, &auth_role, islands.as_ref(), &event) {
                        state_for_stream
                            .metrics
                            .sse_unauthorized_filtered
                            .fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("message").to_string();
                    yield Ok(Event::default().event(event_type).data(event.to_string()));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    state_for_stream.metrics.sse_client_dropped.fetch_add(1, Ordering::Relaxed);
                    emit_server_event(&state_for_stream, "SSE_CLIENT_DROPPED", json!({"site": site, "skipped": skipped}));
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }

        state_for_stream.metrics.sse_active.fetch_sub(1, Ordering::Relaxed);
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(20))
            .text("keepalive"),
    )
}

fn publish_backend_event(state: &Arc<AppState>, event: &Value) {
    let reason = event
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("backend-event");
    let site = event
        .get("site")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if let Some(island_id) = event.get("islandId").and_then(|v| v.as_str()) {
        broadcast_app_event(
            state,
            json!({
                "type": "island.invalidate",
                "reason": reason,
                "site": site,
                "islandId": island_id,
                "parameters": event.get("parameters").cloned().unwrap_or_else(|| json!({})),
                "timestamp": event.get("timestamp").cloned().unwrap_or_else(|| json!(now_millis())),
                "cursor": event.get("cursor").cloned().unwrap_or_else(|| json!(state.store.latest_cursor())),
                "dataContract": event.get("dataContract").cloned().unwrap_or(Value::Null),
                "payload": event.get("payload").cloned().unwrap_or_else(|| json!({})),
            }),
        );
    }

    if let Some(intents) = event.get("intents").and_then(|v| v.as_array()) {
        broadcast_app_event(
            state,
            json!({
                "type": "invalidate",
                "reason": reason,
                "site": site,
                "cursor": event.get("cursor").cloned().unwrap_or_else(|| json!(state.store.latest_cursor())),
                "intents": intents,
            }),
        );
    }
}

fn broadcast_app_event(state: &Arc<AppState>, event: Value) {
    state
        .metrics
        .broadcast_count
        .fetch_add(1, Ordering::Relaxed);
    let _ = state.events.send(event);
}

fn teardown_connection_state(state: &Arc<AppState>, connection_id: &str) {
    state.metrics.ws_active.fetch_sub(1, Ordering::Relaxed);
    let mut registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");
    registry.remove(connection_id);
}

fn register_channel_subscription(
    state: &Arc<AppState>,
    connection_id: &str,
    site: &str,
    channel: &str,
    params: Value,
) {
    let mut registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");
    let row = registry.entry(connection_id.to_string()).or_default();
    row.site = site.to_string();
    let subscription_key = subscription_key(channel, &params);
    row.subscriptions.insert(
        subscription_key,
        ChannelSubscription {
            channel: channel.to_string(),
            params,
        },
    );
}

fn unregister_channel_subscription(
    state: &Arc<AppState>,
    connection_id: &str,
    channel: &str,
    params: &Value,
) {
    let mut registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");
    if let Some(row) = registry.get_mut(connection_id) {
        row.subscriptions.remove(&subscription_key(channel, params));
        if row.subscriptions.is_empty() {
            registry.remove(connection_id);
        }
    }
}

fn build_frames_for_connection(
    state: &Arc<AppState>,
    context: &ConnectionContext,
    event: &Value,
) -> Vec<Value> {
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("message");
    if event_type == "island.invalidate" {
        if is_island_authorized(state, &context.auth_role, None, event) {
            return vec![event.clone()];
        }
        state
            .metrics
            .ws_unauthorized_filtered
            .fetch_add(1, Ordering::Relaxed);
        return Vec::new();
    }

    if event_type != "invalidate" {
        return Vec::new();
    }

    let row = {
        let registry = state
            .channel_registry
            .lock()
            .expect("channel registry lock");
        match registry.get(&context.connection_id) {
            Some(row) => row.clone(),
            None => return Vec::new(),
        }
    };

    let event_site = event
        .get("site")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if event_site != row.site {
        return Vec::new();
    }

    let event_channels = extract_event_channels(event);
    if event_channels.is_empty() {
        return Vec::new();
    }

    let intents = event.get("intents").cloned().unwrap_or_else(|| json!([]));
    let reason = event
        .get("reason")
        .cloned()
        .unwrap_or_else(|| json!("backend-event"));
    let cursor = event
        .get("cursor")
        .cloned()
        .unwrap_or_else(|| json!(state.store.latest_cursor()));
    let site = row.site.clone();

    row.subscriptions
        .values()
        .filter(|subscription| {
            event_channels
                .iter()
                .any(|channel_id| channel_id == &subscription.channel)
        })
        .map(|subscription| {
            json!({
                "type": "channel.invalidate",
                "site": site.clone(),
                "channel": subscription.channel.clone(),
                "params": subscription.params.clone(),
                "reason": reason.clone(),
                "cursor": cursor.clone(),
                "intents": intents.clone(),
            })
        })
        .collect()
}

fn store_entries_for_channel_after(
    state: &Arc<AppState>,
    channel: &str,
    cursor: u64,
    limit: usize,
) -> Vec<IntentRecord> {
    state
        .store
        .entries_after(cursor, limit.saturating_mul(4).max(limit))
        .into_iter()
        .filter(|entry| entry_matches_channel(entry, channel))
        .take(limit)
        .collect()
}

fn entry_matches_channel(entry: &IntentRecord, channel: &str) -> bool {
    let channels = entry
        .meta
        .get("channels")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
        });

    match channels {
        Some(values) if !values.is_empty() => values.iter().any(|value| *value == channel),
        _ => channel == "global",
    }
}

fn extract_event_channels(event: &Value) -> Vec<String> {
    if let Some(intents) = event.get("intents").and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for intent in intents {
            if let Some(channels) = intent
                .get("meta")
                .and_then(|m| m.get("channels"))
                .and_then(|v| v.as_array())
            {
                for channel in channels {
                    if let Some(name) = channel.as_str() {
                        out.push(name.to_string());
                    }
                }
            }
        }
        if out.is_empty() {
            out.push("global".to_string());
        }
        return out;
    }
    vec!["global".to_string()]
}

fn api_error(status: StatusCode, code: &str) -> ApiError {
    (status, Json(json!({ "error": code })))
}

fn multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    let message = error.to_string().to_lowercase();
    if message.contains("length limit") || message.contains("body too large") {
        api_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE")
    } else {
        api_error(StatusCode::BAD_REQUEST, "INVALID_MULTIPART")
    }
}

fn request_site(headers: &HeaderMap) -> String {
    headers
        .get("x-ssma-site")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

fn media_type_from_mime(mime: &str) -> Option<&'static str> {
    if mime.starts_with("image/") {
        Some("image")
    } else if mime.starts_with("audio/") {
        Some("audio")
    } else {
        None
    }
}

fn asset_metadata(record: &AssetRecord) -> AssetMetadata {
    AssetMetadata {
        asset_id: record.asset_id.clone(),
        site: record.site.clone(),
        media_type: record.media_type.clone(),
        mime_type: record.mime_type.clone(),
        file_name: record.file_name.clone(),
        size_bytes: record.size_bytes,
        created_at: record.created_at_secs,
        expires_at: record.expires_at_secs,
    }
}

fn internal_asset(state: &Arc<AppState>, asset_id: &str) -> ApiResult<AssetRecord> {
    let assets = state.assets.lock().expect("assets lock");
    assets
        .get(asset_id)
        .cloned()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "ASSET_NOT_FOUND"))
}

fn owned_asset(
    state: &Arc<AppState>,
    asset_id: &str,
    site: &str,
    actor_key: &str,
) -> ApiResult<AssetRecord> {
    let assets = state.assets.lock().expect("assets lock");
    let Some(record) = assets.get(asset_id).cloned() else {
        return Err(api_error(StatusCode::NOT_FOUND, "ASSET_NOT_FOUND"));
    };
    if record.site != site {
        return Err(api_error(StatusCode::FORBIDDEN, "ASSET_SITE_MISMATCH"));
    }
    if record.owner_key != actor_key {
        return Err(api_error(StatusCode::FORBIDDEN, "ASSET_ACCESS_DENIED"));
    }
    Ok(record)
}

fn ensure_backend_token(headers: &HeaderMap, config: &Config) -> ApiResult<()> {
    if config.backend_internal_token.is_empty() {
        return Ok(());
    }
    let token = headers
        .get("x-ssma-backend-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if token == config.backend_internal_token {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED_BACKEND_REQUEST",
        ))
    }
}

fn resolve_actor_from_headers(
    headers: &HeaderMap,
    config: &Config,
    issue_if_missing: bool,
) -> Option<ActorIdentity> {
    if let Some(user) = resolve_user_from_headers(headers, config) {
        return Some(ActorIdentity {
            actor_key: format!("user:{}", user.id),
            actor_id: user.id.clone(),
            role: user.role.clone(),
            user_id: Some(user.id),
            set_cookie: None,
        });
    }

    if let Some(anon) = cookie_value(headers, &config.anonymous_cookie_name) {
        return Some(ActorIdentity {
            actor_key: format!("anon:{}", anon),
            actor_id: anon,
            role: "guest".to_string(),
            user_id: None,
            set_cookie: None,
        });
    }

    if !issue_if_missing {
        return None;
    }

    let anon = Uuid::new_v4().to_string();
    Some(ActorIdentity {
        actor_key: format!("anon:{}", anon),
        actor_id: anon.clone(),
        role: "guest".to_string(),
        user_id: None,
        set_cookie: Some(format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax",
            config.anonymous_cookie_name, anon
        )),
    })
}

fn purge_expired_runtime_state(state: &Arc<AppState>) {
    let now = now_secs();
    {
        let mut assets = state.assets.lock().expect("assets lock");
        let mut expired = Vec::new();
        for (asset_id, record) in assets.iter() {
            if record.expires_at_secs <= now {
                expired.push((asset_id.clone(), record.path.clone()));
            }
        }
        for (asset_id, path) in expired {
            assets.remove(&asset_id);
            let _ = fs::remove_file(path);
        }
    }
    {
        let mut sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
        sessions.retain(|_, session| session.expires_at_secs > now);
    }
}

fn rtc_channel_name(session_id: &str) -> String {
    format!("rtc.session.{}", session_id)
}

fn rtc_signal_event_intent(session_id: &str, signal: &RtcSignalRecord, channel: &str) -> Value {
    json!({
        "id": format!("rtc:{}:{}", session_id, signal.seq),
        "intent": "RTC_SIGNAL",
        "payload": {
            "sessionId": session_id,
            "kind": signal.kind,
            "senderId": signal.sender_id,
            "targetId": signal.target_id,
            "payload": signal.payload,
            "createdAt": signal.created_at_ms,
            "seq": signal.seq,
        },
        "meta": {
            "channels": [channel],
            "ephemeral": true,
        },
        "insertedAt": signal.created_at_ms,
        "logSeq": signal.seq,
    })
}

fn rtc_snapshot_for_channel(
    state: &Arc<AppState>,
    channel: &str,
    cursor: u64,
) -> Option<(Vec<Value>, u64)> {
    let session_id = channel.strip_prefix("rtc.session.")?;
    let sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
    let session = sessions.get(session_id)?;
    let intents = session
        .signals
        .iter()
        .filter(|signal| signal.seq > cursor)
        .map(|signal| rtc_signal_event_intent(&session.session_id, signal, channel))
        .collect::<Vec<_>>();
    let next = session
        .signals
        .last()
        .map(|signal| signal.seq)
        .unwrap_or(cursor);
    Some((intents, next))
}

fn resolve_user_from_headers(headers: &HeaderMap, config: &Config) -> Option<ResolvedUser> {
    let token = cookie_value(headers, &config.auth_cookie_name)?;
    let claims = decode::<AuthClaims>(
        &token,
        &DecodingKey::from_secret(config.auth_jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .ok()?
    .claims;

    Some(ResolvedUser {
        id: claims.sub,
        role: claims.role.unwrap_or_else(|| "user".to_string()),
    })
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get("cookie")?.to_str().ok()?;
    for segment in cookie.split(';') {
        let trimmed = segment.trim();
        if let Some(value) = trimmed.strip_prefix(&format!("{}=", name)) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn connection_ip_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn parse_requested_islands(raw: Option<&str>) -> Option<Vec<String>> {
    let islands = raw
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    if islands.is_empty() {
        None
    } else {
        Some(islands)
    }
}

fn is_sse_event_authorized(
    state: &Arc<AppState>,
    role: &str,
    requested_islands: Option<&Vec<String>>,
    event: &Value,
) -> bool {
    if extract_event_channels(event)
        .iter()
        .any(|channel| channel.starts_with("rtc.session."))
    {
        return false;
    }
    is_island_authorized(state, role, requested_islands, event)
}

fn is_island_authorized(
    state: &Arc<AppState>,
    role: &str,
    requested_islands: Option<&Vec<String>>,
    event: &Value,
) -> bool {
    if event.get("type").and_then(|value| value.as_str()) != Some("island.invalidate") {
        return true;
    }
    let Some(island_id) = event.get("islandId").and_then(|value| value.as_str()) else {
        return false;
    };
    if let Some(allowed_islands) = requested_islands {
        if !allowed_islands.iter().any(|value| value == island_id) {
            return false;
        }
    }
    let required_role = state
        .config
        .island_access
        .get(island_id)
        .map(|value| value.as_str())
        .unwrap_or("guest");
    role_rank(role) >= role_rank(required_role)
}

fn subscription_key(channel: &str, params: &Value) -> String {
    format!("{}:{}", channel, stable_value_string(params))
}

fn stable_value_string(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let serialized = items
                .iter()
                .map(stable_value_string)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{}]", serialized)
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let serialized = entries
                .into_iter()
                .map(|(key, item)| format!("\"{}\":{}", key, stable_value_string(item)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", serialized)
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    }
}

fn normalize_status(status: Option<&str>) -> &'static str {
    match status.unwrap_or("failed").to_lowercase().as_str() {
        "acked" => "acked",
        "rejected" => "rejected",
        "conflict" => "conflict",
        "failed" => "failed",
        _ => "failed",
    }
}

fn consume_channel_rate_limit(
    state: &Arc<AppState>,
    key: String,
    max: u32,
    window_ms: i64,
) -> bool {
    let now = now_millis();
    let mut buckets = state.channel_limits.lock().expect("channel limit lock");
    let bucket = buckets.entry(key).or_insert(RateBucket {
        count: 0,
        expires_at_ms: now + window_ms,
    });
    if bucket.expires_at_ms < now {
        bucket.count = 0;
        bucket.expires_at_ms = now + window_ms;
    }
    bucket.count += 1;
    bucket.count <= max
}

fn consume_global_rate_limit(state: &Arc<AppState>, key: String, max: u32, window_ms: i64) -> bool {
    let now = now_millis();
    let mut buckets = state.global_limits.lock().expect("global limit lock");
    let bucket = buckets.entry(key).or_insert(RateBucket {
        count: 0,
        expires_at_ms: now + window_ms,
    });
    if bucket.expires_at_ms < now {
        bucket.count = 0;
        bucket.expires_at_ms = now + window_ms;
    }
    bucket.count += 1;
    bucket.count <= max
}

fn can_access_channel(state: &Arc<AppState>, channel: &str, context: &ConnectionContext) -> bool {
    if let Some(session_id) = channel.strip_prefix("rtc.session.") {
        let Some(actor_key) = &context.actor_key else {
            return false;
        };
        let sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
        let Some(session) = sessions.get(session_id) else {
            return false;
        };
        return session.site == context.site
            && (session.owner_key == *actor_key
                || session
                    .participants
                    .iter()
                    .any(|participant| participant == actor_key));
    }
    if !state
        .config
        .protected_channels
        .iter()
        .any(|name| name == channel)
    {
        return true;
    }
    role_rank(&context.auth_role) >= role_rank(&state.config.protected_channel_min_role)
}

fn role_rank(role: &str) -> u8 {
    match role {
        "guest" => 0,
        "user" => 1,
        "staff" => 2,
        "admin" => 3,
        "system" => 4,
        _ => 0,
    }
}

fn emit_server_event(state: &Arc<AppState>, event_name: &str, payload: Value) {
    {
        let mut counters = state
            .metrics
            .server_events
            .lock()
            .expect("server events lock");
        let value = counters.entry(event_name.to_string()).or_insert(0);
        *value += 1;
    }
    tracing::info!(event_name = event_name, payload = %payload, "ssma.server_event");
}

fn subprotocol_major_match(expected: &str, actual: &str) -> bool {
    let e = expected.split('.').next().unwrap_or("0");
    let a = actual.split('.').next().unwrap_or("0");
    e == a
}
