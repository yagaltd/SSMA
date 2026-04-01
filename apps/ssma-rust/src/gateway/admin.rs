use crate::gateway::{
    api_error, resolve_user_from_headers, role_rank, ApiResult, AppState, ResolvedUser,
};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct IntentsQuery {
    reason: Option<String>,
    limit: Option<usize>,
}

fn require_staff_role(headers: &HeaderMap, state: &AppState) -> ApiResult<ResolvedUser> {
    let user = resolve_user_from_headers(headers, &state.config)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    if role_rank(&user.role) < role_rank("staff") {
        return Err(api_error(StatusCode::FORBIDDEN, "INSUFFICIENT_ROLE"));
    }
    Ok(user)
}

pub(crate) async fn admin_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let _user = require_staff_role(&headers, &state)?;

    let registry = state
        .channel_registry
        .lock()
        .expect("channel registry lock");

    // channel_name -> { count, sites }
    let mut channels: BTreeMap<String, ChannelSummary> = BTreeMap::new();

    for connection in registry.values() {
        let mut seen_channels = BTreeSet::new();
        for sub in connection.subscriptions.values() {
            if seen_channels.insert(sub.channel.clone()) {
                let entry = channels.entry(sub.channel.clone()).or_default();
                entry.count += 1;
                entry.sites.insert(connection.site.clone());
            }
        }
    }

    let channels_json: Value = channels
        .into_iter()
        .map(|(name, summary)| {
            (
                name,
                json!({
                    "count": summary.count,
                    "sites": summary.sites.into_iter().collect::<Vec<_>>(),
                }),
            )
        })
        .collect();

    Ok(Json(json!({ "channels": channels_json })))
}

pub(crate) async fn admin_intents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<IntentsQuery>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_staff_role(&headers, &state)?;

    let limit = params.limit.unwrap_or(200).min(500).max(1);

    // Fetch more than needed to allow filtering, but cap at a reasonable multiplier
    let fetch_limit = if params.reason.is_some() {
        limit * 4
    } else {
        limit
    };
    let entries = state.store.entries_after(0, fetch_limit);

    let filtered: Vec<IntentRecordJson> = entries
        .into_iter()
        .filter(|entry| {
            if let Some(ref reason) = params.reason {
                entry.intent == *reason
            } else {
                true
            }
        })
        .take(limit)
        .map(|entry| IntentRecordJson {
            id: entry.id,
            intent: entry.intent,
            payload: entry.payload,
            meta: entry.meta,
            inserted_at: entry.inserted_at,
            log_seq: entry.log_seq,
            site: entry.site,
            status: entry.status,
            connection_id: entry.connection_id,
            backend: entry.backend,
        })
        .collect();

    let count = filtered.len();
    let cursor = filtered.iter().map(|r| r.log_seq).max().unwrap_or(0);

    Ok(Json(json!({
        "intents": filtered,
        "count": count,
        "cursor": cursor,
    })))
}

struct ChannelSummary {
    count: u64,
    sites: BTreeSet<String>,
}

impl Default for ChannelSummary {
    fn default() -> Self {
        Self {
            count: 0,
            sites: BTreeSet::new(),
        }
    }
}

#[derive(serde::Serialize)]
struct IntentRecordJson {
    id: String,
    intent: String,
    payload: Value,
    meta: Value,
    inserted_at: i64,
    log_seq: u64,
    site: String,
    status: String,
    connection_id: Option<String>,
    backend: Option<Value>,
}
