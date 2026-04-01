use crate::gateway::{
    api_error, connection_ip_from_headers, consume_global_rate_limit, ApiResult, AppState,
};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct LogsBatchRequest {
    logs: Vec<Value>,
    site: Option<String>,
}

pub(crate) async fn logs_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<LogsBatchRequest>,
) -> ApiResult<impl IntoResponse> {
    let ip = connection_ip_from_headers(&headers);
    if !consume_global_rate_limit(&state, format!("logs:{}", ip), 60, 60_000) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"));
    }

    if state.config.log_relay_url.is_empty() {
        return Ok(Json(json!({ "status": "disabled" })));
    }

    let payload = json!({
        "logs": body.logs,
        "site": body.site.unwrap_or_default(),
        "gatewayMeta": {
            "timestamp": crate::runtime::now_millis(),
        }
    });

    let count = body.logs.len();
    let result = reqwest::Client::new()
        .post(&state.config.log_relay_url)
        .json(&payload)
        .timeout(std::time::Duration::from_millis(state.config.backend_timeout_ms))
        .send()
        .await;

    match result {
        Ok(_) => Ok(Json(json!({ "status": "ok", "forwarded": count }))),
        Err(e) => Ok(Json(json!({ "status": "relay_failed", "error": e.to_string() }))),
    }
}

pub(crate) async fn logs_health(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    Json(json!({
        "status": if state.config.log_relay_url.is_empty() { "disabled" } else { "active" },
        "relayUrl": if state.config.log_relay_url.is_empty() { Value::Null } else { Value::String(state.config.log_relay_url.clone()) },
    }))
}
