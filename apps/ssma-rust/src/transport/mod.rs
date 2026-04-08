pub mod admin;
pub mod auth;
pub mod internal;
pub mod sse;
pub mod ws;

use crate::config::Config;
use crate::domain::runtime::{now_millis, now_secs, IntentRecord, IntentStore};
use crate::adapters::backend::{BackendHttpClient, BackendUser};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use std::time::Duration;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

// --- Types ---

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub store: IntentStore,
    pub backend: BackendHttpClient,
    pub events: broadcast::Sender<Value>,
    pub(crate) user_store: Arc<auth::UserStore>,
    pub(crate) assets: Arc<Mutex<HashMap<String, AssetRecord>>>,
    pub(crate) rtc_sessions: Arc<Mutex<HashMap<String, RtcSessionRecord>>>,
    pub(crate) channel_limits: Arc<Mutex<HashMap<String, RateBucket>>>,
    pub(crate) global_limits: Arc<Mutex<HashMap<String, RateBucket>>>,
    pub(crate) channel_registry: Arc<Mutex<HashMap<String, ConnectionChannels>>>,
    pub(crate) metrics: Arc<MetricsState>,
    pub(crate) log_client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub(crate) struct AssetRecord {
    pub(crate) asset_id: String,
    pub(crate) site: String,
    pub(crate) owner_key: String,
    pub(crate) media_type: String,
    pub(crate) mime_type: String,
    pub(crate) file_name: Option<String>,
    pub(crate) size_bytes: u64,
    pub(crate) path: PathBuf,
    pub(crate) created_at_secs: u64,
    pub(crate) expires_at_secs: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RtcSignalRecord {
    pub(crate) seq: u64,
    pub(crate) kind: String,
    pub(crate) sender_id: String,
    pub(crate) target_id: Option<String>,
    pub(crate) payload: Value,
    pub(crate) created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct RtcSessionRecord {
    pub(crate) session_id: String,
    pub(crate) site: String,
    pub(crate) owner_key: String,
    pub(crate) participants: Vec<String>,
    pub(crate) signals: Vec<RtcSignalRecord>,
    pub(crate) next_seq: u64,
    pub(crate) expires_at_secs: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActorIdentity {
    pub(crate) actor_key: String,
    pub(crate) actor_id: String,
    pub(crate) role: String,
    pub(crate) user_id: Option<String>,
    pub(crate) set_cookie: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetMetadata {
    pub(crate) asset_id: String,
    pub(crate) site: String,
    pub(crate) media_type: String,
    pub(crate) mime_type: String,
    pub(crate) file_name: Option<String>,
    pub(crate) size_bytes: u64,
    pub(crate) created_at: u64,
    pub(crate) expires_at: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RateBucket {
    pub(crate) count: u32,
    pub(crate) expires_at_ms: i64,
}

#[derive(Debug, Default)]
pub(crate) struct MetricsState {
    pub(crate) ws_active: AtomicU64,
    pub(crate) sse_active: AtomicU64,
    pub(crate) ws_total: AtomicU64,
    pub(crate) sse_total: AtomicU64,
    pub(crate) broadcast_count: AtomicU64,
    pub(crate) rate_limit_hits: AtomicU64,
    pub(crate) sse_client_dropped: AtomicU64,
    pub(crate) ws_unauthorized_filtered: AtomicU64,
    pub(crate) sse_unauthorized_filtered: AtomicU64,
    pub(crate) server_events: Mutex<HashMap<String, u64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelSubscription {
    pub(crate) channel: String,
    pub(crate) params: Value,
    pub(crate) subscribed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionUserSummary {
    pub(crate) id: String,
    pub(crate) role: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ConnectionChannels {
    pub(crate) site: String,
    pub(crate) connection_role: String,
    pub(crate) user: Option<ConnectionUserSummary>,
    pub(crate) subscriptions: HashMap<String, ChannelSubscription>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WsQuery {
    pub(crate) role: Option<String>,
    pub(crate) site: Option<String>,
    pub(crate) subprotocol: Option<String>,
    pub(crate) cursor: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SseQuery {
    pub(crate) site: Option<String>,
    pub(crate) cursor: Option<u64>,
    pub(crate) islands: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BackendEventsPayload {
    pub(crate) events: Option<Vec<Value>>,
    pub(crate) event: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PublicQueryRequest {
    pub(crate) payload: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnectionContext {
    pub(crate) transport_role: String,
    pub(crate) auth_role: String,
    pub(crate) site: String,
    pub(crate) connection_id: String,
    pub(crate) actor_key: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) ip: String,
    pub(crate) user_agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AuthClaims {
    pub(crate) sub: String,
    pub(crate) role: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedUser {
    pub(crate) id: String,
    pub(crate) role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRtcSessionRequest {
    pub(crate) participants: Option<Vec<String>>,
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PostRtcSignalRequest {
    pub(crate) kind: String,
    pub(crate) sender_id: String,
    pub(crate) target_id: Option<String>,
    pub(crate) payload: Value,
}

pub(crate) type ApiError = (StatusCode, Json<Value>);
pub(crate) type ApiResult<T> = Result<T, ApiError>;

// --- Top-level functions ---

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
    let store = IntentStore::new(config.intent_store_path.clone(), config.replay_window_ms, config.optimistic_max_entries);
    let backend = BackendHttpClient::with_timeout(config.backend_url.clone(), config.backend_timeout_ms);
    let (events, _) = broadcast::channel(1024);
    let user_store = Arc::new(auth::UserStore::new(config.user_store_path.clone()));
    Arc::new(AppState {
        config,
        store,
        backend,
        events,
        user_store,
        assets: Arc::new(Mutex::new(HashMap::new())),
        rtc_sessions: Arc::new(Mutex::new(HashMap::new())),
        channel_limits: Arc::new(Mutex::new(HashMap::new())),
        global_limits: Arc::new(Mutex::new(HashMap::new())),
        channel_registry: Arc::new(Mutex::new(HashMap::new())),
        metrics: Arc::new(MetricsState::default()),
        log_client: reqwest::Client::new(),
    })
}

pub fn app(state: Arc<AppState>) -> Router {
    // Short timeout for standard HTTP requests (5 seconds)
    let short_timeout = Duration::from_millis(state.config.backend_timeout_ms);
    // Longer timeout for long-lived operations like initial subscription (30 seconds)
    let long_timeout = Duration::from_secs(30);

    // Routes with short timeout
    let short_timeout_routes = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/query/:name", post(public_query))
        .route("/query/:name/stream", post(public_query_stream))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/media/assets", post(crate::features::media::upload_media))
        .route(
            "/media/assets/:asset_id",
            get(crate::features::media::get_asset_metadata)
                .delete(crate::features::media::delete_asset),
        )
        .route(
            "/media/assets/:asset_id/content",
            get(crate::features::media::get_asset_content),
        )
        .route("/rtc/sessions", post(crate::features::rtc::create_rtc_session))
        .route(
            "/rtc/sessions/:session_id/signals",
            post(crate::features::rtc::post_rtc_signal),
        )
        .route("/logs/batch", post(crate::features::logs::logs_batch))
        .route("/logs/health", get(crate::features::logs::logs_health))
        .route("/optimistic/metrics", get(metrics))
        .route(
            "/optimistic/rework",
            post(crate::features::optimistic::optimistic_rework),
        )
        .route(
            "/optimistic/undo",
            post(crate::features::optimistic::optimistic_undo),
        )
        .route(
            "/optimistic/pending",
            get(crate::features::optimistic::optimistic_pending),
        )
        .route("/admin/optimistic/channels", get(admin::admin_channels))
        .route("/admin/optimistic/intents", get(admin::admin_intents))
        .route("/internal/backend/events", post(internal::backend_events_ingest))
        .route("/internal/assets", post(internal::create_internal_asset))
        .route(
            "/internal/assets/:asset_id",
            get(internal::get_internal_asset_metadata).delete(internal::delete_internal_asset),
        )
        .route(
            "/internal/assets/:asset_id/content",
            get(internal::get_internal_asset_content),
        )
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::new(short_timeout))
                .layer(DefaultBodyLimit::max(
                    state.config.media_max_upload_bytes as usize,
                ))
                .layer(cors_layer(&state.config.allowed_origins)),
        )
        .with_state(state.clone());

    // Routes with long timeout for WebSocket and SSE upgrade
    let long_timeout_routes = Router::new()
        .route("/optimistic/ws", get(ws::ws_upgrade))
        .route("/optimistic/events", get(sse::sse_events))
        .layer(
            ServiceBuilder::new()
                .layer(TimeoutLayer::new(long_timeout)),
        )
        .with_state(state);

    // Combine all routes
    short_timeout_routes.merge(long_timeout_routes)
}

fn cors_layer(allowed_origins: &str) -> CorsLayer {
    if allowed_origins == "*" {
        CorsLayer::very_permissive()
    } else {
        let origins: Vec<_> = allowed_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let state = build_state(config.clone());
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await?;
    tracing::info!(addr = %listener.local_addr()?, "ssma-rust listening");

    serve_with_shutdown(listener, state, tokio::signal::ctrl_c()).await
}

pub async fn serve_with_shutdown(
    listener: tokio::net::TcpListener,
    state: Arc<AppState>,
    signal: impl std::future::Future<Output = std::io::Result<()>> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = app(state.clone());

    // Wrap the signal future to broadcast shutdown to WS/SSE clients
    let state_for_signal = state.clone();
    let shutdown_trigger = async move {
        let _ = signal.await;
        tracing::info!("ssma-rust shutdown signal received, draining connections");
        let _ = state_for_signal.events.send(json!({
            "type": "server.shutdown",
            "reason": "graceful",
            "message": "Server is shutting down"
        }));
    };

    // with_graceful_shutdown stops accepting new connections once the signal fires.
    // The broadcast `server.shutdown` event tells WS/SSE loops to close,
    // which allows the server to finish.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_trigger)
        .await?;

    // After server finishes, persist intent store
    if let Err(e) = state.store.flush_to_disk() {
        tracing::error!(%e, "ssma-rust failed to persist intent store");
    } else {
        tracing::info!("ssma-rust intent store persisted");
    }

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

async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Check backend connectivity if configured
    let backend_status = if state.backend.is_configured() {
        match state.backend.health(&crate::adapters::backend::BackendContext {
            site: "internal".to_string(),
            actor_key: None,
            connection_id: None,
            ip: None,
            user_agent: None,
            user: None,
        }).await {
            Ok(health) => {
                let status = health.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                if status == "ok" { "healthy" } else { "degraded" }
            }
            Err(_) => "unreachable",
        }
    } else {
        "unconfigured"
    };

    let is_ready = backend_status != "unreachable";
    let status_code = if is_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(json!({
        "status": if is_ready { "ok" } else { "not_ready" },
        "service": "ssma-rust",
        "subprotocol": state.config.subprotocol,
        "cursor": state.store.latest_cursor(),
        "backend": backend_status,
    })))
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

pub(crate) fn collect_gateway_metrics(state: &AppState) -> Value {
    json!({
        "active": {
            "ws": state.metrics.ws_active.load(Ordering::Relaxed),
            "sse": state.metrics.sse_active.load(Ordering::Relaxed),
        },
        "totals": {
            "wsConnections": state.metrics.ws_total.load(Ordering::Relaxed),
            "sseConnections": state.metrics.sse_total.load(Ordering::Relaxed),
            "broadcasts": state.metrics.broadcast_count.load(Ordering::Relaxed),
            "rateLimitHits": state.metrics.rate_limit_hits.load(Ordering::Relaxed),
        },
        "serverEvents": state.metrics.server_events.lock().expect("server events lock").clone(),
    })
}

async fn public_query(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<PublicQueryRequest>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let backend_ctx = crate::adapters::backend::BackendContext {
        site,
        actor_key: Some(actor.actor_key.clone()),
        connection_id: None,
        ip: Some(connection_ip_from_headers(&headers)),
        user_agent: headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string()),
        user: Some(BackendUser {
            id: actor.user_id.clone(),
            role: actor.role.clone(),
        }),
    };

    let response = state
        .backend
        .query(&name, body.payload, &backend_ctx)
        .await
        .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "BACKEND_QUERY_FAILED"))?;

    let normalized = match response {
        Value::Object(ref object) if object.contains_key("status") => response,
        other => json!({ "status": "ok", "data": other }),
    };

    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }

    Ok((StatusCode::OK, response_headers, Json(normalized)))
}

async fn public_query_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<PublicQueryRequest>,
) -> ApiResult<impl IntoResponse> {
    use futures_util::StreamExt;
    use std::convert::Infallible;

    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let backend_ctx = crate::adapters::backend::BackendContext {
        site,
        actor_key: Some(actor.actor_key.clone()),
        connection_id: None,
        ip: Some(connection_ip_from_headers(&headers)),
        user_agent: headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string()),
        user: Some(BackendUser {
            id: actor.user_id.clone(),
            role: actor.role.clone(),
        }),
    };

    let stream = state
        .backend
        .query_stream(&name, body.payload, &backend_ctx)
        .await
        .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "BACKEND_STREAM_FAILED"))?;

    let sse_stream = stream.map(|result| -> Result<axum::response::sse::Event, Infallible> {
        match result {
            Ok(data) => Ok(axum::response::sse::Event::default()
                .json_data(&data)
                .unwrap_or_else(|_| axum::response::sse::Event::default().data("{}"))),
            Err(_) => Ok(axum::response::sse::Event::default()
                .event("error")
                .data("{\"error\":\"STREAM_ERROR\"}")),
        }
    });

    let sse = axum::response::Sse::new(sse_stream);

    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }

    Ok((StatusCode::OK, response_headers, sse))
}

// --- Shared utility functions (pub(crate)) ---

pub(crate) fn api_error(status: StatusCode, code: &str) -> ApiError {
    (status, Json(json!({ "error": code })))
}

pub(crate) fn multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    let message = error.to_string().to_lowercase();
    if message.contains("length limit") || message.contains("body too large") {
        api_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE")
    } else {
        api_error(StatusCode::BAD_REQUEST, "INVALID_MULTIPART")
    }
}

pub(crate) fn request_site(headers: &HeaderMap) -> String {
    headers
        .get("x-ssma-site")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn connection_ip_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(crate) fn resolve_actor_from_headers(
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

pub(crate) fn resolve_user_from_headers(headers: &HeaderMap, config: &Config) -> Option<ResolvedUser> {
    let token = cookie_value(headers, &config.auth_cookie_name)?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    let claims = decode::<AuthClaims>(
        &token,
        &DecodingKey::from_secret(config.auth_jwt_secret.as_bytes()),
        &validation,
    )
    .ok()?
    .claims;

    Some(ResolvedUser {
        id: claims.sub,
        role: claims.role.unwrap_or_else(|| "user".to_string()),
    })
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
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

pub(crate) fn emit_server_event(state: &Arc<AppState>, event_name: &str, payload: Value) {
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

pub(crate) fn broadcast_app_event(state: &Arc<AppState>, event: Value) {
    state
        .metrics
        .broadcast_count
        .fetch_add(1, Ordering::Relaxed);
    let _ = state.events.send(event);
}

pub(crate) fn purge_expired_runtime_state(state: &Arc<AppState>) {
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

pub(crate) fn consume_global_rate_limit(state: &Arc<AppState>, key: String, max: u32, window_ms: i64) -> bool {
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

pub(crate) fn consume_channel_rate_limit(
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

pub(crate) fn role_rank(role: &str) -> u8 {
    match role {
        "guest" => 0,
        "user" => 1,
        "staff" => 2,
        "admin" => 3,
        "system" => 4,
        _ => 0,
    }
}

pub(crate) fn can_access_channel(state: &Arc<AppState>, channel: &str, context: &ConnectionContext) -> bool {
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

pub(crate) fn normalize_status(status: Option<&str>) -> &'static str {
    match status.unwrap_or("failed").to_lowercase().as_str() {
        "acked" => "acked",
        "rejected" => "rejected",
        "conflict" => "conflict",
        "failed" => "failed",
        _ => "failed",
    }
}

pub(crate) fn subscription_key(channel: &str, params: &Value) -> String {
    format!("{}:{}", channel, stable_value_string(params))
}

pub(crate) fn stable_value_string(value: &Value) -> String {
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

pub(crate) fn reason_list(meta: &Value) -> Vec<String> {
    meta.get("reasons")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn set_reason_list(meta: &mut Value, reasons: &[String]) {
    if !meta.is_object() {
        *meta = json!({});
    }
    if let Some(object) = meta.as_object_mut() {
        object.insert(
            "reasons".to_string(),
            Value::Array(
                reasons
                    .iter()
                    .map(|reason| Value::String(reason.clone()))
                    .collect(),
            ),
        );
    }
}

pub(crate) fn ensure_reason(meta: &mut Value, reason: &str) {
    let mut reasons = reason_list(meta);
    if !reasons.iter().any(|value| value == reason) {
        reasons.push(reason.to_string());
    }
    set_reason_list(meta, &reasons);
}

pub(crate) fn remove_reason(meta: &mut Value, reason: &str) {
    let mut reasons = reason_list(meta);
    reasons.retain(|value| value != reason);
    set_reason_list(meta, &reasons);
}

pub(crate) fn normalize_intent_meta(meta: &Value) -> Value {
    let mut normalized = if meta.is_object() {
        meta.clone()
    } else {
        json!({})
    };

    let channels = normalized
        .get("channels")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| vec!["global".to_string()]);

    let mut reasons = reason_list(&normalized);
    if !reasons.iter().any(|value| value == "pending") {
        reasons.push("pending".to_string());
    }
    if !reasons.iter().any(|value| value == "replay") {
        reasons.push("replay".to_string());
    }
    for channel in channels {
        let reason = format!("channel:{}", channel);
        if !reasons.iter().any(|value| value == &reason) {
            reasons.push(reason);
        }
    }
    set_reason_list(&mut normalized, &reasons);
    normalized
}

pub(crate) fn consume_actor_rate_limit(
    state: &Arc<AppState>,
    bucket_name: &str,
    actor_key: &str,
    max: u32,
    window_ms: i64,
) -> bool {
    consume_global_rate_limit(
        state,
        format!("{}:{}", bucket_name, actor_key),
        max,
        window_ms,
    )
}

pub(crate) fn ensure_backend_token(headers: &HeaderMap, config: &Config) -> ApiResult<()> {
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

pub(crate) fn extract_event_channels(event: &Value) -> Vec<String> {
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

pub(crate) fn entry_matches_channel(entry: &IntentRecord, channel: &str) -> bool {
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

pub(crate) fn store_entries_for_channel_after(
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

pub(crate) fn is_island_authorized(
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

pub(crate) fn subprotocol_major_match(expected: &str, actual: &str) -> bool {
    let e = expected.split('.').next().unwrap_or("0");
    let a = actual.split('.').next().unwrap_or("0");
    e == a
}

pub(crate) fn asset_metadata(record: &AssetRecord) -> AssetMetadata {
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

pub(crate) fn owned_asset(
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

pub(crate) fn teardown_connection_state(state: &Arc<AppState>, connection_id: &str) {
    state.metrics.ws_active.fetch_sub(1, Ordering::Relaxed);
    let mut registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");
    registry.remove(connection_id);
}

pub(crate) fn register_channel_subscription(
    state: &Arc<AppState>,
    connection_id: &str,
    site: &str,
    connection_role: &str,
    user: Option<ConnectionUserSummary>,
    channel: &str,
    params: Value,
) {
    let mut registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");
    let row = registry.entry(connection_id.to_string()).or_default();
    row.site = site.to_string();
    row.connection_role = connection_role.to_string();
    row.user = user;
    let sub_key = subscription_key(channel, &params);
    row.subscriptions.insert(
        sub_key,
        ChannelSubscription {
            channel: channel.to_string(),
            params,
            subscribed_at: now_millis(),
        },
    );
}

pub(crate) fn unregister_channel_subscription(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_rank_ordering() {
        assert!(role_rank("guest") < role_rank("user"));
        assert!(role_rank("user") < role_rank("staff"));
        assert!(role_rank("staff") < role_rank("admin"));
        assert!(role_rank("admin") < role_rank("system"));
    }

    #[test]
    fn role_rank_unknown_defaults_to_zero() {
        assert_eq!(role_rank("unknown"), 0);
        assert_eq!(role_rank(""), 0);
        assert_eq!(role_rank("superadmin"), 0);
    }

    #[test]
    fn normalize_status_known_values() {
        assert_eq!(normalize_status(Some("acked")), "acked");
        assert_eq!(normalize_status(Some("rejected")), "rejected");
        assert_eq!(normalize_status(Some("conflict")), "conflict");
        assert_eq!(normalize_status(Some("failed")), "failed");
    }

    #[test]
    fn normalize_status_defaults_to_failed() {
        assert_eq!(normalize_status(None), "failed");
        assert_eq!(normalize_status(Some("")), "failed");
        assert_eq!(normalize_status(Some("unknown")), "failed");
    }

    #[test]
    fn subprotocol_major_match_exact() {
        assert!(subprotocol_major_match("1.0.0", "1.2.3"));
        assert!(subprotocol_major_match("1.0.0", "1.0.0"));
    }

    #[test]
    fn subprotocol_major_match_rejects_mismatch() {
        assert!(!subprotocol_major_match("1.0.0", "2.0.0"));
        assert!(!subprotocol_major_match("2.0.0", "1.9.9"));
    }

    #[test]
    fn normalize_intent_meta_adds_default_reasons() {
        let meta = json!({});
        let normalized = normalize_intent_meta(&meta);
        // Should add reasons including pending, replay, and channel:global
        let reasons = normalized.get("reasons").and_then(|v| v.as_array()).unwrap();
        let reason_strs: Vec<&str> = reasons.iter().filter_map(|r| r.as_str()).collect();
        assert!(reason_strs.contains(&"pending"));
        assert!(reason_strs.contains(&"replay"));
        assert!(reason_strs.contains(&"channel:global"));
    }

    #[test]
    fn normalize_intent_meta_preserves_existing_channels() {
        let meta = json!({"channels": ["custom-channel"]});
        let normalized = normalize_intent_meta(&meta);
        let channels = normalized.get("channels").and_then(|v| v.as_array()).unwrap();
        assert!(channels.iter().any(|c| c.as_str() == Some("custom-channel")));
        let reasons = normalized.get("reasons").and_then(|v| v.as_array()).unwrap();
        let reason_strs: Vec<&str> = reasons.iter().filter_map(|r| r.as_str()).collect();
        assert!(reason_strs.contains(&"channel:custom-channel"));
    }
}
