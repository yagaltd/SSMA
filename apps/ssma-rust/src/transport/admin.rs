use crate::gateway::{
    api_error, resolve_user_from_headers, role_rank, ApiResult, AppState,
    ConnectionChannels,
};
use crate::features::optimistic;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct IntentsQuery {
    reason: Option<String>,
    limit: Option<usize>,
}

fn require_staff_role(headers: &HeaderMap, state: &AppState) -> ApiResult<crate::gateway::ResolvedUser> {
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
        .expect("channel registry lock")
        .clone();

    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut total_subscriptions = 0usize;
    for (connection_id, row) in registry {
        collect_connection_subscriptions(&mut grouped, &connection_id, &row, &mut total_subscriptions);
    }

    let channels = grouped
        .into_iter()
        .map(|(channel, subscribers)| {
            json!({
                "channel": channel,
                "total": subscribers.len(),
                "subscribers": subscribers,
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "updatedAt": crate::runtime::now_millis(),
        "totalSubscriptions": total_subscriptions,
        "channels": channels,
    })))
}

pub(crate) async fn admin_intents(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<IntentsQuery>,
) -> ApiResult<impl IntoResponse> {
    let _user = require_staff_role(&headers, &state)?;

    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let entries = optimistic::pending_entries(&state, params.reason.as_deref(), limit);

    let mut reason_summary = BTreeMap::<String, u64>::new();
    for entry in &entries {
        for reason in crate::gateway::reason_list(&entry.meta) {
            *reason_summary.entry(reason).or_insert(0) += 1;
        }
    }

    Ok(Json(json!({
        "updatedAt": crate::runtime::now_millis(),
        "pending": entries.iter().map(optimistic::pending_record_summary).collect::<Vec<_>>(),
        "reasonSummary": reason_summary.into_iter().map(|(reason, count)| json!({ "reason": reason, "count": count })).collect::<Vec<_>>(),
        "total": entries.len(),
    })))
}

fn collect_connection_subscriptions(
    grouped: &mut BTreeMap<String, Vec<Value>>,
    connection_id: &str,
    row: &ConnectionChannels,
    total_subscriptions: &mut usize,
) {
    for subscription in row.subscriptions.values() {
        *total_subscriptions += 1;
        grouped
            .entry(subscription.channel.clone())
            .or_default()
            .push(json!({
                "connectionId": connection_id,
                "params": subscription.params,
                "subscribedAt": subscription.subscribed_at,
                "connectionRole": row.connection_role,
                "site": row.site,
                "user": row.user,
            }));
    }
}
