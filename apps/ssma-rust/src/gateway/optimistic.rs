use crate::gateway::{
    api_error, broadcast_app_event, consume_actor_rate_limit, ensure_reason, reason_list,
    remove_reason, resolve_user_from_headers, role_rank, ApiResult, AppState,
};
use crate::runtime::IntentRecord;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct ReworkRequest {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoRequest {
    pub(crate) id: String,
    pub(crate) site: String,
    pub(crate) intent: String,
    pub(crate) payload: Value,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PendingQuery {
    pub(crate) since: Option<u64>,
    pub(crate) limit: Option<usize>,
    pub(crate) site: Option<String>,
}

fn require_staff(
    headers: &HeaderMap,
    config: &crate::config::Config,
) -> ApiResult<crate::gateway::ResolvedUser> {
    let user = resolve_user_from_headers(headers, config)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    if role_rank(&user.role) < role_rank("staff") {
        return Err(api_error(StatusCode::FORBIDDEN, "INSUFFICIENT_ROLE"));
    }
    Ok(user)
}

fn require_user(
    headers: &HeaderMap,
    config: &crate::config::Config,
) -> ApiResult<crate::gateway::ResolvedUser> {
    resolve_user_from_headers(headers, config)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))
}

fn apply_rework_limit(state: &Arc<AppState>, actor_key: &str) -> ApiResult<()> {
    if consume_actor_rate_limit(
        state,
        "optimistic-rework",
        actor_key,
        state.config.optimistic_rework_max,
        state.config.optimistic_rework_window_ms as i64,
    ) {
        Ok(())
    } else {
        state
            .metrics
            .rate_limit_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(api_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"))
    }
}

fn entry_channels(entry: &IntentRecord) -> Vec<String> {
    entry
        .meta
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
        .unwrap_or_else(|| vec!["global".to_string()])
}

fn entry_has_reason(entry: &IntentRecord, reason: &str) -> bool {
    reason_list(&entry.meta).iter().any(|value| value == reason)
}

fn pending_record_json(entry: &IntentRecord) -> Value {
    json!({
        "id": entry.id,
        "intent": entry.intent,
        "channels": entry_channels(entry),
        "reasons": reason_list(&entry.meta),
        "site": entry.site,
        "status": entry.status,
        "connectionId": entry.connection_id,
        "insertedAt": entry.inserted_at,
    })
}

pub(crate) async fn optimistic_rework(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ReworkRequest>,
) -> ApiResult<impl IntoResponse> {
    let user = require_staff(&headers, &state.config)?;
    apply_rework_limit(&state, &format!("{}:{}", user.id, user.role))?;

    let record = state
        .store
        .get(&body.id, &body.site)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;
    let undo = record
        .meta
        .get("undo")
        .cloned()
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "UNDO_NOT_AVAILABLE"))?;
    let channels = entry_channels(&record);
    let reason = body.reason.unwrap_or_else(|| "manual-rework".to_string());

    let updated = state
        .store
        .update_entry(&body.id, &body.site, |entry| {
            ensure_reason(&mut entry.meta, "rework");
        })
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;

    broadcast_app_event(
        &state,
        json!({
            "type": "rework",
            "reason": reason,
            "site": updated.site,
            "intents": [{
                "id": updated.id,
                "intent": updated.intent,
                "payload": updated.payload,
                "undo": undo,
                "channels": channels,
                "site": updated.site,
            }]
        }),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "scheduled",
            "id": body.id,
        })),
    ))
}

pub(crate) async fn optimistic_undo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<UndoRequest>,
) -> ApiResult<impl IntoResponse> {
    let user = require_user(&headers, &state.config)?;
    apply_rework_limit(&state, &format!("{}:{}", user.id, user.role))?;

    let record = state
        .store
        .get(&body.id, &body.site)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;
    let expected = record
        .meta
        .get("undo")
        .cloned()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;
    if expected.get("intent").and_then(Value::as_str) != Some(body.intent.as_str())
        || expected.get("payload").cloned().unwrap_or(Value::Null) != body.payload
    {
        return Err(api_error(StatusCode::CONFLICT, "UNDO_MISMATCH"));
    }
    if let Some(owner_id) = record.user_id.as_deref() {
        if owner_id != user.id {
            return Err(api_error(StatusCode::FORBIDDEN, "UNDO_NOT_OWNER"));
        }
    }

    let channels = entry_channels(&record);
    let reason = body.reason.unwrap_or_else(|| "client-undo".to_string());

    let updated = state
        .store
        .update_entry(&body.id, &body.site, |entry| {
            remove_reason(&mut entry.meta, "pending");
            remove_reason(&mut entry.meta, "replay");
            remove_reason(&mut entry.meta, "rework");
            for channel in entry_channels(entry) {
                remove_reason(&mut entry.meta, &format!("channel:{}", channel));
            }
            entry.status = "undone".to_string();
        })
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "INTENT_NOT_FOUND"))?;

    broadcast_app_event(
        &state,
        json!({
            "type": "undo",
            "id": updated.id,
            "intent": updated.intent,
            "site": updated.site,
            "reason": reason,
        }),
    );
    broadcast_app_event(
        &state,
        json!({
            "type": "invalidate",
            "reason": "intent-undo",
            "site": updated.site,
            "cursor": state.store.latest_cursor(),
            "intents": [{
                "id": updated.id,
                "intent": updated.intent,
                "payload": updated.payload,
                "meta": updated.meta,
                "insertedAt": updated.inserted_at,
                "logSeq": updated.log_seq,
                "channels": channels,
            }],
        }),
    );

    Ok(Json(json!({
        "status": "reverted",
        "id": body.id,
    })))
}

pub(crate) async fn optimistic_pending(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PendingQuery>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_user(&headers, &state.config)?;

    let since = query.since.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).clamp(1, 500);

    let mut entries = state.store.entries_after(since, limit.saturating_mul(4));
    entries.retain(|entry| !reason_list(&entry.meta).is_empty());
    if let Some(ref site) = query.site {
        entries.retain(|entry| entry.site == *site);
    }
    entries.truncate(limit);

    let count = entries.len();
    let cursor = entries.last().map(|entry| entry.log_seq).unwrap_or(since);

    Ok(Json(json!({
        "pending": entries.iter().map(pending_record_json).collect::<Vec<_>>(),
        "count": count,
        "cursor": cursor,
    })))
}

pub(crate) fn pending_entries(
    state: &Arc<AppState>,
    reason_filter: Option<&str>,
    limit: usize,
) -> Vec<IntentRecord> {
    let mut entries = state.store.entries_after(0, limit.saturating_mul(8).max(limit));
    entries.retain(|entry| !reason_list(&entry.meta).is_empty());
    if let Some(reason) = reason_filter {
        entries.retain(|entry| entry_has_reason(entry, reason));
    }
    entries.truncate(limit);
    entries
}

pub(crate) fn pending_record_summary(entry: &IntentRecord) -> Value {
    pending_record_json(entry)
}
