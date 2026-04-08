use crate::adapters::backend::{BackendContext, BackendUser};
use crate::transport::{
    api_error, connection_ip_from_headers, consume_global_rate_limit, emit_server_event,
    purge_expired_runtime_state, request_site, resolve_actor_from_headers, ApiResult, AppState,
};
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FormSubmitRequest {
    form_name: String,
    payload: Value,
    honeypot: Option<String>,
    captcha_token: Option<String>,
    meta: Option<Value>,
}

pub(crate) async fn submit_form(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<FormSubmitRequest>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);

    if body.form_name.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_FORM_NAME"));
    }
    if !body.payload.is_object() {
        return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_FORM_PAYLOAD"));
    }

    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let ip = connection_ip_from_headers(&headers);
    let ua = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    if !consume_global_rate_limit(
        &state,
        format!("forms:{}:{}", site, ip),
        state.config.form_rate_max,
        state.config.form_rate_window_ms,
    ) {
        emit_server_event(
            &state,
            "FORM_RATE_LIMITED",
            json!({"site": site, "ip": ip, "formName": body.form_name}),
        );
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"));
    }

    if body
        .honeypot
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        emit_server_event(
            &state,
            "FORM_HONEYPOT_BLOCKED",
            json!({"site": site, "formName": body.form_name, "actorKey": actor.actor_key}),
        );

        let mut response_headers = HeaderMap::new();
        if let Some(cookie) = actor.set_cookie {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response_headers.insert(SET_COOKIE, value);
            }
        }

        return Ok((
            StatusCode::ACCEPTED,
            response_headers,
            Json(json!({
                "status": "accepted",
                "dropped": true,
                "reason": "honeypot",
            })),
        ));
    }

    let captcha_verified = verify_captcha(
        &state,
        body.captcha_token.as_deref(),
        &body.form_name,
        &site,
        &ip,
        ua.as_deref(),
        &actor.actor_key,
    )
    .await?;
    if !captcha_verified {
        emit_server_event(
            &state,
            "FORM_CAPTCHA_REJECTED",
            json!({"site": site, "formName": body.form_name, "actorKey": actor.actor_key}),
        );
        return Err(api_error(StatusCode::FORBIDDEN, "CAPTCHA_VERIFICATION_FAILED"));
    }

    let backend_ctx = BackendContext {
        site: site.clone(),
        actor_key: Some(actor.actor_key.clone()),
        connection_id: None,
        ip: Some(ip.clone()),
        user_agent: ua,
        user: actor.user_id.clone().map(|id| BackendUser {
            id: Some(id),
            role: actor.role.clone(),
        }),
    };

    let response = state
        .backend
        .submit_form(
            &body.form_name,
            body.payload,
            body.meta.unwrap_or_else(|| json!({})),
            &backend_ctx,
        )
        .await
        .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "BACKEND_FORM_SUBMIT_FAILED"))?;

    emit_server_event(
        &state,
        "FORM_SUBMITTED",
        json!({"site": site, "formName": body.form_name, "actorKey": actor.actor_key}),
    );

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

async fn verify_captcha(
    state: &Arc<AppState>,
    token: Option<&str>,
    form_name: &str,
    site: &str,
    ip: &str,
    user_agent: Option<&str>,
    actor_key: &str,
) -> ApiResult<bool> {
    match state.config.form_captcha_mode.as_str() {
        "disabled" => Ok(true),
        "external" => {
            let Some(token) = token else {
                return Err(api_error(StatusCode::BAD_REQUEST, "CAPTCHA_REQUIRED"));
            };

            let response = state
                .log_client
                .post(&state.config.form_captcha_verify_url)
                .timeout(std::time::Duration::from_millis(
                    state.config.form_captcha_timeout_ms,
                ))
                .json(&json!({
                    "token": token,
                    "formName": form_name,
                    "site": site,
                    "ip": ip,
                    "userAgent": user_agent,
                    "actorKey": actor_key,
                }))
                .send()
                .await
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "CAPTCHA_VERIFICATION_FAILED"))?;

            if !response.status().is_success() {
                return Ok(false);
            }

            let body = response
                .json::<Value>()
                .await
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "CAPTCHA_VERIFICATION_FAILED"))?;

            Ok(body.get("ok").and_then(Value::as_bool).unwrap_or(false))
        }
        _ => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INVALID_CAPTCHA_MODE",
        )),
    }
}
