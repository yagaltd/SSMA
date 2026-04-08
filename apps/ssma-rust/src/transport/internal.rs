use super::{
    api_error, asset_metadata, emit_server_event, ensure_backend_token, multipart_error,
    purge_expired_runtime_state, ApiResult, AppState, AssetRecord,
    BackendEventsPayload,
};
use crate::features::audio;
use crate::runtime::{now_millis, now_secs};
use axum::extract::{Multipart, Path, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn publish_backend_event(state: &Arc<AppState>, event: &Value) {
    if let Some(event_type) = event.get("eventType").and_then(|v| v.as_str()) {
        if let Some(audio_session_id) = event.get("audioSessionId").and_then(|v| v.as_str()) {
            let site = event
                .get("site")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let payload = event.get("payload").cloned().unwrap_or_else(|| json!({}));
            if let Some(broadcast) =
                audio::record_backend_audio_event(state, &site, audio_session_id, event_type, payload)
            {
                super::broadcast_app_event(state, broadcast);
            }
            return;
        }
    }
    let reason = event
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("backend-event");
    let site = event
        .get("site")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if let Some(island_id) = event.get("islandId").and_then(|v| v.as_str()) {
        super::broadcast_app_event(
            state,
            json!({
                "type": "island.invalidate",
                "reason": reason,
                "site": site,
                "islandId": island_id,
                "parameters": event.get("parameters").cloned().unwrap_or_else(|| json!({})),
                "timestamp": event.get("timestamp").cloned().unwrap_or_else(|| json!(now_millis())),
                "cursor": event.get("cursor").cloned().unwrap_or_else(|| json!(state.store.latest_cursor())),
                "dataContract": event.get("dataContract").cloned().unwrap_or(Value::Null),
                "payload": event.get("payload").cloned().unwrap_or_else(|| json!({})),
            }),
        );
    }

    if let Some(intents) = event.get("intents").and_then(|v| v.as_array()) {
        super::broadcast_app_event(
            state,
            json!({
                "type": "invalidate",
                "reason": reason,
                "site": site,
                "cursor": event.get("cursor").cloned().unwrap_or_else(|| json!(state.store.latest_cursor())),
                "intents": intents,
            }),
        );
    }
}

fn internal_asset(state: &Arc<AppState>, asset_id: &str) -> ApiResult<AssetRecord> {
    let assets = state.assets.lock().expect("assets lock");
    assets
        .get(asset_id)
        .cloned()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "ASSET_NOT_FOUND"))
}

pub(crate) async fn backend_events_ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<BackendEventsPayload>,
) -> impl IntoResponse {
    if !state.config.backend_internal_token.is_empty() {
        let token = headers
            .get("x-ssma-backend-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if token != state.config.backend_internal_token {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "UNAUTHORIZED_BACKEND_EVENT_SOURCE" })),
            );
        }
    }

    let mut processed = 0usize;
    let mut events = body.events.unwrap_or_default();
    if let Some(event) = body.event {
        events.push(event);
    }
    for event in events {
        processed += 1;
        publish_backend_event(&state, &event);
    }

    (
        StatusCode::ACCEPTED,
        Json(json!({ "status": "accepted", "processed": processed })),
    )
}

pub(crate) async fn create_internal_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;

    let mut site: Option<String> = None;
    let mut actor_key: Option<String> = None;
    let mut media_type: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut file_name: Option<String> = None;
    let mut ttl_secs: Option<u64> = None;
    let mut selected: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("file") => {
                if file_name.is_none() {
                    file_name = field.file_name().map(|value| value.to_string());
                }
                if mime_type.is_none() {
                    mime_type = field.content_type().map(|value| value.to_string());
                }
                let bytes = field.bytes().await.map_err(multipart_error)?;
                if bytes.is_empty() {
                    continue;
                }
                selected = Some(bytes.to_vec());
            }
            Some("site") => {
                site = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .trim()
                        .to_string(),
                );
            }
            Some("actorKey") => {
                actor_key = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .trim()
                        .to_string(),
                );
            }
            Some("mediaType") => {
                media_type = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .trim()
                        .to_string(),
                );
            }
            Some("mimeType") => {
                mime_type = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .trim()
                        .to_string(),
                );
            }
            Some("fileName") => {
                file_name = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_error)?
                        .trim()
                        .to_string(),
                );
            }
            Some("ttlSecs") => {
                let value = field.text().await.map_err(multipart_error)?;
                ttl_secs = value.trim().parse::<u64>().ok();
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = selected.ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "MISSING_MEDIA"))?;
    if bytes.len() as u64 > state.config.media_max_upload_bytes {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
        ));
    }

    let site = site
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "MISSING_SITE"))?;
    let actor_key = actor_key
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "MISSING_ACTOR_KEY"))?;
    let media_type = media_type
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "MISSING_MEDIA_TYPE"))?;
    let mime_type = mime_type
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "MISSING_MIME_TYPE"))?;

    let asset_id = Uuid::new_v4().to_string();
    let path = state
        .config
        .media_storage_root
        .join(format!("{}.bin", &asset_id));
    std::fs::write(&path, &bytes)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "ASSET_WRITE_FAILED"))?;

    let now = now_secs();
    let record = AssetRecord {
        asset_id: asset_id.clone(),
        site: site.clone(),
        owner_key: actor_key,
        media_type: media_type.clone(),
        mime_type,
        file_name,
        size_bytes: bytes.len() as u64,
        path,
        created_at_secs: now,
        expires_at_secs: now + ttl_secs.unwrap_or(state.config.media_ttl_secs),
    };
    let metadata = asset_metadata(&record);
    state
        .assets
        .lock()
        .expect("assets lock")
        .insert(asset_id.clone(), record);

    emit_server_event(
        &state,
        "MEDIA_ASSET_CREATED",
        json!({"assetId": asset_id, "site": site, "mediaType": media_type}),
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({ "status": "ok", "asset": metadata })),
    ))
}

pub(crate) async fn get_internal_asset_metadata(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;
    let record = internal_asset(&state, &asset_id)?;
    Ok(Json(
        json!({ "status": "ok", "asset": asset_metadata(&record) }),
    ))
}

pub(crate) async fn get_internal_asset_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;
    let record = internal_asset(&state, &asset_id)?;
    let bytes = std::fs::read(&record.path)
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "ASSET_CONTENT_MISSING"))?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&record.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, response_headers, bytes))
}

pub(crate) async fn delete_internal_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    ensure_backend_token(&headers, &state.config)?;
    let record = internal_asset(&state, &asset_id)?;
    {
        let mut assets = state.assets.lock().expect("assets lock");
        assets.remove(&record.asset_id);
    }
    let _ = std::fs::remove_file(&record.path);
    emit_server_event(
        &state,
        "MEDIA_ASSET_DELETED",
        json!({"assetId": asset_id, "site": record.site}),
    );
    Ok(Json(json!({ "status": "ok", "deleted": true })))
}
