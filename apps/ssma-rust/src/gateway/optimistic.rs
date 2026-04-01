use crate::gateway::{
    api_error, broadcast_app_event, resolve_user_from_headers, role_rank, ApiResult, AppState,
};
use crate::runtime::now_millis;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

// --- Types ---

#[derive(Debug, Deserialize)]
pub(crate) struct ReworkRequest {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) meta: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UndoRequest {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) undo_payload: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PendingQuery {
    pub(crate) since: Option<u64>,
    pub(crate) limit: Option<usize>,
    pub(crate) site: Option<String>,
}

// --- Auth helpers ---

fn require_staff(headers: &HeaderMap, config: &crate::config::Config) -> ApiResult<crate::gateway::ResolvedUser> {
    let user = resolve_user_from_headers(headers, config)
        .ok_or_else(|| api_error(axum::http::StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    if role_rank(&user.role) < role_rank("staff") {
        return Err(api_error(axum::http::StatusCode::FORBIDDEN, "INSUFFICIENT_ROLE"));
    }
    Ok(user)
}

fn require_user(headers: &HeaderMap, config: &crate::config::Config) -> ApiResult<crate::gateway::ResolvedUser> {
    resolve_user_from_headers(headers, config)
        .ok_or_else(|| api_error(axum::http::StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))
}

// --- Handlers ---

pub(crate) async fn optimistic_rework(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ReworkRequest>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_staff(&headers, &state.config)?;

    // Validate meta.undo exists
    if body.meta.get("undo").is_none() {
        return Err(api_error(
            axum::http::StatusCode::BAD_REQUEST,
            "META_UNDO_REQUIRED",
        ));
    }

    // Find the intent
    let record = state
        .store
        .get(&body.id, &body.site)
        .ok_or_else(|| api_error(axum::http::StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;

    // Check rework window
    let now = now_millis();
    let window = state.config.optimistic_rework_window_ms as i64;
    if record.inserted_at < now - window {
        return Err(api_error(
            axum::http::StatusCode::BAD_REQUEST,
            "REWORK_WINDOW_EXPIRED",
        ));
    }

    // Update status to "reworked"
    state
        .store
        .update_status(&body.id, &body.site, "reworked", None);

    // Broadcast event
    broadcast_app_event(
        &state,
        json!({
            "type": "optimistic.rework",
            "intentId": body.id,
            "site": body.site,
            "meta": body.meta,
        }),
    );

    Ok(Json(json!({
        "status": "ok",
        "intentId": body.id,
    })))
}

pub(crate) async fn optimistic_undo(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<UndoRequest>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_user(&headers, &state.config)?;

    // Validate undoPayload is present and is an object
    if !body.undo_payload.is_object() {
        return Err(api_error(
            axum::http::StatusCode::BAD_REQUEST,
            "UNDO_PAYLOAD_REQUIRED",
        ));
    }

    // Find the intent
    let record = state
        .store
        .get(&body.id, &body.site)
        .ok_or_else(|| api_error(axum::http::StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;

    // Check rework window
    let now = now_millis();
    let window = state.config.optimistic_rework_window_ms as i64;
    if record.inserted_at < now - window {
        return Err(api_error(
            axum::http::StatusCode::BAD_REQUEST,
            "REWORK_WINDOW_EXPIRED",
        ));
    }

    // Update status to "undone"
    state
        .store
        .update_status(&body.id, &body.site, "undone", None);

    // Broadcast event
    broadcast_app_event(
        &state,
        json!({
            "type": "optimistic.undo",
            "intentId": body.id,
            "site": body.site,
            "undoPayload": body.undo_payload,
        }),
    );

    Ok(Json(json!({
        "status": "ok",
        "intentId": body.id,
    })))
}

pub(crate) async fn optimistic_pending(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PendingQuery>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_user(&headers, &state.config)?;

    let since = query.since.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(500);

    let mut entries = state.store.entries_after(since, limit);

    // Filter by status "pending"
    entries.retain(|entry| entry.status == "pending");

    // Filter by site if provided
    if let Some(ref site) = query.site {
        entries.retain(|entry| entry.site == *site);
    }

    let count = entries.len();
    let cursor = state.store.latest_cursor();

    Ok(Json(json!({
        "intents": entries,
        "count": count,
        "cursor": cursor,
    })))
}
