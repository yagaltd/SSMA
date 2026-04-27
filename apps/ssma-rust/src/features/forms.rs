use crate::adapters::backend::{BackendContext, BackendUser};
use crate::transport::{
    api_error, connection_ip_from_headers, consume_global_rate_limit, cookie_value,
    emit_server_event, purge_expired_runtime_state, request_id_from_headers, request_site,
    resolve_actor_from_headers, ApiResult, AppState,
};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
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
    body: Bytes,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    if body.len() as u64 > state.config.form_max_body_bytes {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
        ));
    }
    let request_id = request_id_from_headers(&headers);
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let body = parse_form_request(&headers, &content_type, &body).await?;

    if body.form_name.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_FORM_NAME"));
    }
    if !body.payload.is_object() {
        return Err(api_error(StatusCode::BAD_REQUEST, "INVALID_FORM_PAYLOAD"));
    }

    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    enforce_csrf_if_needed(&state, &headers, &body, &content_type)?;
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
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response_headers.insert("x-request-id", value);
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
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "CAPTCHA_VERIFICATION_FAILED",
        ));
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
            Some(&request_id),
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
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response_headers.insert("x-request-id", value);
    }

    Ok((StatusCode::OK, response_headers, Json(normalized)))
}

pub(crate) async fn verify_captcha(
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
            match state.config.form_captcha_adapter.as_str() {
                "external-json" => {
                    verify_external_json_captcha(
                        state, token, form_name, site, ip, user_agent, actor_key,
                    )
                    .await
                }
                "cap-siteverify" => verify_cap_siteverify_captcha(state, token).await,
                _ => Err(api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INVALID_CAPTCHA_ADAPTER",
                )),
            }
        }
        _ => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INVALID_CAPTCHA_MODE",
        )),
    }
}

async fn verify_external_json_captcha(
    state: &Arc<AppState>,
    token: &str,
    form_name: &str,
    site: &str,
    ip: &str,
    user_agent: Option<&str>,
    actor_key: &str,
) -> ApiResult<bool> {
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

async fn verify_cap_siteverify_captcha(state: &Arc<AppState>, token: &str) -> ApiResult<bool> {
    if state.config.form_captcha_secret.is_empty() {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "CAPTCHA_SECRET_MISSING",
        ));
    }

    let response = state
        .log_client
        .post(&state.config.form_captcha_verify_url)
        .timeout(std::time::Duration::from_millis(
            state.config.form_captcha_timeout_ms,
        ))
        .json(&json!({
            "secret": state.config.form_captcha_secret,
            "response": token,
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

    Ok(body
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

async fn parse_form_request(
    headers: &HeaderMap,
    content_type: &str,
    body: &Bytes,
) -> ApiResult<FormSubmitRequest> {
    if content_type.contains("application/json") || content_type.is_empty() {
        return serde_json::from_slice::<FormSubmitRequest>(body)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_JSON"));
    }
    if content_type.contains("application/x-www-form-urlencoded") {
        let parsed = serde_urlencoded::from_bytes::<HashMap<String, String>>(body)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_FORM_URLENCODED"))?;
        let form_name = parsed.get("formName").cloned().unwrap_or_default();
        let honeypot = parsed.get("honeypot").cloned();
        let captcha_token = parsed.get("captchaToken").cloned();
        let meta = parsed
            .get("meta")
            .and_then(|value| serde_json::from_str::<Value>(value).ok());
        let payload = Value::Object(
            parsed
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "formName" | "honeypot" | "captchaToken" | "meta" | "csrfToken"
                    )
                })
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect(),
        );
        return Ok(FormSubmitRequest {
            form_name,
            payload,
            honeypot,
            captcha_token,
            meta,
        });
    }
    if content_type.contains("multipart/form-data") {
        let boundary = multer::parse_boundary(content_type)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_MULTIPART"))?;
        let one_shot = stream::once(async { Ok::<Bytes, Infallible>(body.clone()) });
        let mut multipart = multer::Multipart::new(one_shot, boundary);

        let mut fields = HashMap::<String, String>::new();
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_MULTIPART"))?
        {
            let name = match field.name() {
                Some(name) => name.to_string(),
                None => continue,
            };
            if field.file_name().is_some() {
                continue;
            }
            let value = field
                .text()
                .await
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "INVALID_MULTIPART"))?;
            fields.insert(name, value);
        }

        let form_name = fields.get("formName").cloned().unwrap_or_default();
        let honeypot = fields.get("honeypot").cloned();
        let captcha_token = fields.get("captchaToken").cloned();
        let meta = fields
            .get("meta")
            .and_then(|value| serde_json::from_str::<Value>(value).ok());
        let payload = Value::Object(
            fields
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "formName" | "honeypot" | "captchaToken" | "meta" | "csrfToken"
                    )
                })
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect(),
        );
        return Ok(FormSubmitRequest {
            form_name,
            payload,
            honeypot,
            captcha_token,
            meta,
        });
    }
    let _ = headers;
    Err(api_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "UNSUPPORTED_FORM_CONTENT_TYPE",
    ))
}

fn enforce_csrf_if_needed(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: &FormSubmitRequest,
    content_type: &str,
) -> ApiResult<()> {
    if state.config.form_csrf_mode != "double-submit" {
        return Ok(());
    }
    if !content_type.contains("application/x-www-form-urlencoded") {
        return Ok(());
    }
    let cookie = cookie_value(headers, &state.config.form_csrf_cookie_name)
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "CSRF_TOKEN_MISSING"))?;
    let token = headers
        .get(&state.config.form_csrf_header_name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or_else(|| {
            body.meta
                .as_ref()
                .and_then(|meta| meta.get("csrfToken"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let token = token.ok_or_else(|| api_error(StatusCode::FORBIDDEN, "CSRF_TOKEN_MISSING"))?;
    if token != cookie {
        return Err(api_error(StatusCode::FORBIDDEN, "CSRF_TOKEN_INVALID"));
    }
    Ok(())
}
