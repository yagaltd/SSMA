use crate::transport::{
    api_error, collect_gateway_metrics, connection_ip_from_headers, consume_global_rate_limit,
    ApiResult, AppState,
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
    logs: Option<Vec<Value>>,
    site: Option<String>,
    #[serde(rename = "batchId")]
    batch_id: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    source: Option<String>,
    meta: Option<Value>,
    entries: Option<Vec<Value>>,
}

impl LogsBatchRequest {
    fn forwarded_payload(self, gateway_metrics: Value) -> Value {
        let gateway = json!({
            "timestamp": crate::runtime::now_millis(),
            "metrics": gateway_metrics,
        });

        if let Some(entries) = self.entries {
            return json!({
                "batchId": self.batch_id,
                "sessionId": self.session_id,
                "userId": self.user_id,
                "source": self.source,
                "meta": self.meta.unwrap_or_else(|| json!({})),
                "entries": entries,
                "site": self.site.unwrap_or_default(),
                "gateway": gateway,
            });
        }

        json!({
            "logs": self.logs.unwrap_or_default(),
            "site": self.site.unwrap_or_default(),
            "gateway": gateway,
        })
    }

    fn entry_count(&self) -> usize {
        self.entries
            .as_ref()
            .map(|entries| entries.len())
            .or_else(|| self.logs.as_ref().map(|logs| logs.len()))
            .unwrap_or(0)
    }
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

    let gateway_metrics = collect_gateway_metrics(&state);
    let count = body.entry_count();
    let payload = body.forwarded_payload(gateway_metrics);

    let result = state
        .log_client
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
