use crate::adapters::backend::BackendContext;
use crate::transport::{
    api_error, connection_ip_from_headers, emit_server_event, purge_expired_runtime_state,
    request_id_from_headers, request_site, ApiResult, AppState,
};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn webhook_ingest(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    if body.len() as u64 > state.config.webhook_max_body_bytes {
        return Err(api_error(StatusCode::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE"));
    }
    let request_id = request_id_from_headers(&headers);
    let site = request_site(&headers);
    let ip = connection_ip_from_headers(&headers);
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    let parsed = verify_webhook(&state, &provider, &headers, &body).await?;
    let dedupe_key = format!("{}:{}", provider, parsed.event_id);

    let now = crate::runtime::now_secs();
    {
        let mut seen = state.webhook_seen.lock().expect("webhook seen lock");
        if let Some(expires_at) = seen.get(&dedupe_key) {
            if *expires_at > now {
                let mut out_headers = HeaderMap::new();
                if let Ok(value) = HeaderValue::from_str(&request_id) {
                    out_headers.insert("x-request-id", value);
                }
                return Ok((
                    StatusCode::ACCEPTED,
                    out_headers,
                    Json(json!({"status": "duplicate", "eventId": parsed.event_id})),
                ));
            }
        }
        seen.insert(dedupe_key, now + state.config.webhook_idempotency_ttl_secs);
    }

    let backend_ctx = BackendContext {
        site: site.clone(),
        actor_key: Some(format!("webhook:{}", provider)),
        connection_id: None,
        ip: Some(ip.clone()),
        user_agent,
        user: None,
    };

    let response = state
        .backend
        .ingest_webhook(
            &provider,
            &parsed.event_id,
            &parsed.event_type,
            parsed.payload,
            &backend_ctx,
            Some(&request_id),
        )
        .await
        .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "BACKEND_WEBHOOK_INGEST_FAILED"))?;

    emit_server_event(
        &state,
        "WEBHOOK_INGESTED",
        json!({
            "site": site,
            "provider": provider,
            "eventId": parsed.event_id,
            "eventType": parsed.event_type,
            "requestId": request_id,
        }),
    );

    let normalized = match response {
        Value::Object(ref object) if object.contains_key("status") => response,
        other => json!({ "status": "ok", "data": other }),
    };
    let mut out_headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        out_headers.insert("x-request-id", value);
    }
    Ok((StatusCode::OK, out_headers, Json(normalized)))
}

#[derive(Debug)]
struct ParsedWebhook {
    event_id: String,
    event_type: String,
    payload: Value,
}

async fn verify_webhook(
    state: &Arc<AppState>,
    provider: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> ApiResult<ParsedWebhook> {
    match state.config.webhook_verify_mode.as_str() {
        "disabled" => {
            let json = serde_json::from_slice::<Value>(body)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_WEBHOOK_JSON"))?;
            let event_id = json
                .get("eventId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let event_type = json
                .get("eventType")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let payload = json.get("payload").cloned().unwrap_or_else(|| json.clone());
            if event_id.is_empty() || event_type.is_empty() {
                return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_WEBHOOK_PAYLOAD"));
            }
            Ok(ParsedWebhook {
                event_id,
                event_type,
                payload,
            })
        }
        "external" => {
            let header_map = headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.to_string(), Value::String(value.to_string())))
                })
                .collect::<serde_json::Map<String, Value>>();
            let response = state
                .log_client
                .post(&state.config.webhook_verify_url)
                .timeout(std::time::Duration::from_millis(
                    state.config.webhook_verify_timeout_ms,
                ))
                .json(&json!({
                    "provider": provider,
                    "headers": Value::Object(header_map),
                    "bodyBase64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body),
                }))
                .send()
                .await
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "WEBHOOK_VERIFICATION_FAILED"))?;

            if !response.status().is_success() {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    "WEBHOOK_VERIFICATION_FAILED",
                ));
            }
            let verified = response
                .json::<Value>()
                .await
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "WEBHOOK_VERIFICATION_FAILED"))?;
            if !verified.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    "WEBHOOK_VERIFICATION_FAILED",
                ));
            }
            let event_id = verified
                .get("eventId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let event_type = verified
                .get("eventType")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let payload = verified.get("payload").cloned().unwrap_or(Value::Null);
            if event_id.is_empty() || event_type.is_empty() {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    "WEBHOOK_VERIFICATION_FAILED",
                ));
            }
            Ok(ParsedWebhook {
                event_id,
                event_type,
                payload,
            })
        }
        _ => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INVALID_WEBHOOK_VERIFY_MODE",
        )),
    }
}
