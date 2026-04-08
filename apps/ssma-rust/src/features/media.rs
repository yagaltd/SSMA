use crate::transport::{
    api_error, asset_metadata, emit_server_event, multipart_error, owned_asset,
    purge_expired_runtime_state, request_site, resolve_actor_from_headers,
    ApiResult, AppState, AssetRecord,
};
use crate::runtime::now_secs;
use axum::extract::{Multipart, Path, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

pub(crate) fn media_type_from_mime(mime: &str) -> Option<&'static str> {
    if mime.starts_with("image/") {
        Some("image")
    } else if mime.starts_with("audio/") {
        Some("audio")
    } else {
        None
    }
}

pub(crate) async fn upload_media(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);

    let mut selected: Option<(String, Option<String>, Vec<u8>)> = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let mime = field
            .content_type()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let file_name = field.file_name().map(|value| value.to_string());
        let bytes = field.bytes().await.map_err(multipart_error)?;
        if bytes.is_empty() {
            continue;
        }
        selected = Some((mime, file_name, bytes.to_vec()));
        break;
    }

    let Some((mime_type, file_name, bytes)) = selected else {
        return Err(api_error(StatusCode::BAD_REQUEST, "MISSING_MEDIA"));
    };

    let media_type = media_type_from_mime(&mime_type)
        .ok_or_else(|| api_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE"))?;
    if bytes.len() as u64 > state.config.media_max_upload_bytes {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
        ));
    }

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
        owner_key: actor.actor_key.clone(),
        media_type: media_type.to_string(),
        mime_type,
        file_name,
        size_bytes: bytes.len() as u64,
        path,
        created_at_secs: now,
        expires_at_secs: now + state.config.media_ttl_secs,
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

    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }

    Ok((
        StatusCode::CREATED,
        response_headers,
        Json(json!({ "status": "ok", "asset": metadata })),
    ))
}

pub(crate) async fn get_asset_metadata(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    Ok(Json(
        json!({ "status": "ok", "asset": asset_metadata(&record) }),
    ))
}

pub(crate) async fn get_asset_content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    let file = tokio::fs::File::open(&record.path)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "ASSET_CONTENT_MISSING"))?;
    let metadata = tokio::fs::metadata(&record.path)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "ASSET_CONTENT_MISSING"))?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&record.mime_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&metadata.len().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    Ok((StatusCode::OK, response_headers, body))
}

pub(crate) async fn delete_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let record = owned_asset(&state, &asset_id, &site, &actor.actor_key)?;
    {
        let mut assets = state.assets.lock().expect("assets lock");
        assets.remove(&record.asset_id);
    }
    let _ = std::fs::remove_file(&record.path);
    emit_server_event(
        &state,
        "MEDIA_ASSET_DELETED",
        json!({"assetId": asset_id, "site": site}),
    );
    Ok(Json(json!({ "status": "ok", "deleted": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_from_mime_image() {
        assert_eq!(media_type_from_mime("image/png"), Some("image"));
        assert_eq!(media_type_from_mime("image/jpeg"), Some("image"));
        assert_eq!(media_type_from_mime("image/gif"), Some("image"));
        assert_eq!(media_type_from_mime("image/webp"), Some("image"));
    }

    #[test]
    fn media_type_from_mime_audio() {
        assert_eq!(media_type_from_mime("audio/mpeg"), Some("audio"));
        assert_eq!(media_type_from_mime("audio/ogg"), Some("audio"));
        assert_eq!(media_type_from_mime("audio/wav"), Some("audio"));
    }

    #[test]
    fn media_type_from_mime_rejects_unknown() {
        assert_eq!(media_type_from_mime("video/mp4"), None);
        assert_eq!(media_type_from_mime("application/pdf"), None);
        assert_eq!(media_type_from_mime("text/plain"), None);
        assert_eq!(media_type_from_mime(""), None);
    }
}
