use super::{
    api_error, broadcast_app_event, emit_server_event, now_millis, now_secs,
    purge_expired_runtime_state, request_site, resolve_actor_from_headers, ApiResult,
    AppState, AudioSessionCapabilities, AudioSessionCommandRequest, AudioSessionEventRecord,
    AudioSessionMode, AudioSessionRecord, AudioSessionStatus, CreateAudioSessionRequest,
    RtcSessionRecord,
};
use axum::extract::{Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn audio_channel_name(session_id: &str) -> String {
    format!("audio.session.{}", session_id)
}

pub(crate) fn audio_session_metadata(session: &AudioSessionRecord) -> Value {
    json!({
        "audioSessionId": session.session_id,
        "rtcSessionId": session.rtc_session_id,
        "channel": audio_channel_name(&session.session_id),
        "rtcChannel": super::rtc::rtc_channel_name(&session.rtc_session_id),
        "site": session.site,
        "mode": session.mode,
        "status": session.status,
        "backend": session.backend,
        "capabilities": session.capabilities,
        "participants": session.participants,
        "expiresAt": session.expires_at_secs,
    })
}

pub(crate) fn owned_audio_session(
    state: &Arc<AppState>,
    session_id: &str,
    site: &str,
    actor_key: &str,
) -> ApiResult<AudioSessionRecord> {
    let sessions = state.audio_sessions.lock().expect("audio sessions lock");
    let Some(session) = sessions.get(session_id).cloned() else {
        return Err(api_error(StatusCode::NOT_FOUND, "AUDIO_SESSION_NOT_FOUND"));
    };
    if session.site != site {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "AUDIO_SESSION_SITE_MISMATCH",
        ));
    }
    if session.owner_key != actor_key
        && !session
            .participants
            .iter()
            .any(|participant| participant == actor_key)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "AUDIO_SESSION_ACCESS_DENIED",
        ));
    }
    Ok(session)
}

pub(crate) fn audio_session_for_rtc(
    state: &Arc<AppState>,
    rtc_session_id: &str,
) -> Option<AudioSessionRecord> {
    let sessions = state.audio_sessions.lock().expect("audio sessions lock");
    sessions
        .values()
        .find(|session| session.rtc_session_id == rtc_session_id)
        .cloned()
}

pub(crate) fn append_audio_session_event(
    session: &mut AudioSessionRecord,
    event_type: String,
    payload: Value,
) -> AudioSessionEventRecord {
    session.next_seq += 1;
    let event = AudioSessionEventRecord {
        seq: session.next_seq,
        event_type,
        payload,
        created_at_ms: now_millis(),
    };
    session.events.push(event.clone());
    if session.events.len() > 128 {
        let overflow = session.events.len() - 128;
        session.events.drain(0..overflow);
    }
    event
}

pub fn apply_audio_backend_event(
    state: &Arc<AppState>,
    site: &str,
    session_id: &str,
    event_type: &str,
    payload: Value,
) -> ApiResult<()> {
    if let Some(event) = record_backend_audio_event(state, site, session_id, event_type, payload) {
        broadcast_app_event(state, event);
        Ok(())
    } else {
        Err(api_error(StatusCode::NOT_FOUND, "AUDIO_SESSION_NOT_FOUND"))
    }
}

pub(crate) fn audio_session_event_intent(
    session_id: &str,
    event: &AudioSessionEventRecord,
    channel: &str,
) -> Value {
    json!({
        "id": format!("audio:{}:{}", session_id, event.seq),
        "intent": event.event_type,
        "payload": {
            "audioSessionId": session_id,
            "eventType": event.event_type,
            "payload": event.payload,
            "createdAt": event.created_at_ms,
            "seq": event.seq,
        },
        "meta": {
            "channels": [channel],
            "ephemeral": true,
        },
        "insertedAt": event.created_at_ms,
        "logSeq": event.seq,
    })
}

pub(crate) fn audio_session_broadcast_event(
    site: &str,
    session_id: &str,
    event: &AudioSessionEventRecord,
) -> Value {
    let channel = audio_channel_name(session_id);
    json!({
        "type": "invalidate",
        "reason": "audio-session-event",
        "site": site,
        "cursor": event.seq,
        "intents": [audio_session_event_intent(session_id, event, &channel)],
    })
}

pub(crate) fn audio_snapshot_for_channel(
    state: &Arc<AppState>,
    channel: &str,
    cursor: u64,
) -> Option<(Vec<Value>, u64)> {
    let session_id = channel.strip_prefix("audio.session.")?;
    let sessions = state.audio_sessions.lock().expect("audio sessions lock");
    let session = sessions.get(session_id)?;
    let intents = session
        .events
        .iter()
        .filter(|event| event.seq > cursor)
        .map(|event| audio_session_event_intent(&session.session_id, event, channel))
        .collect::<Vec<_>>();
    let next = session
        .events
        .last()
        .map(|event| event.seq)
        .unwrap_or(cursor);
    Some((intents, next))
}

pub(crate) fn record_backend_audio_event(
    state: &Arc<AppState>,
    site: &str,
    session_id: &str,
    event_type: &str,
    payload: Value,
) -> Option<Value> {
    let event = {
        let mut sessions = state.audio_sessions.lock().expect("audio sessions lock");
        let session = sessions.get_mut(session_id)?;
        let next_status = match event_type {
            "audio.session.started" => Some(AudioSessionStatus::Streaming),
            "audio.session.ended" => Some(AudioSessionStatus::Ended),
            "audio.session.error" => Some(AudioSessionStatus::Error),
            "audio.session.interrupted" => Some(AudioSessionStatus::Paused),
            _ => None,
        };
        if let Some(status) = next_status {
            session.status = status;
        }
        append_audio_session_event(session, event_type.to_string(), payload)
    };
    Some(audio_session_broadcast_event(site, session_id, &event))
}

pub(crate) async fn create_audio_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateAudioSessionRequest>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let session_id = Uuid::new_v4().to_string();
    let rtc_session_id = Uuid::new_v4().to_string();
    let mut participants = vec![actor.actor_key.clone()];
    for participant in body.participants.unwrap_or_default() {
        if !participants.iter().any(|value| value == &participant) {
            participants.push(participant);
        }
    }
    let now = now_secs();
    let ttl = body.ttl_secs.unwrap_or(state.config.media_ttl_secs);
    let expires_at = now + ttl;
    let mode = body.mode.unwrap_or(AudioSessionMode::SpeechToSpeech);
    let capabilities = body.capabilities.unwrap_or(AudioSessionCapabilities {
        audio_in: true,
        audio_out: true,
        partial_transcript: true,
        interrupt: true,
    });
    let backend = body.backend.unwrap_or_else(|| "models_local".to_string());

    let rtc_record = RtcSessionRecord {
        session_id: rtc_session_id.clone(),
        site: site.clone(),
        owner_key: actor.actor_key.clone(),
        participants: participants.clone(),
        signals: Vec::new(),
        next_seq: 0,
        expires_at_secs: expires_at,
    };
    state
        .rtc_sessions
        .lock()
        .expect("rtc sessions lock")
        .insert(rtc_session_id.clone(), rtc_record);

    let audio_record = AudioSessionRecord {
        session_id: session_id.clone(),
        rtc_session_id: rtc_session_id.clone(),
        site: site.clone(),
        owner_key: actor.actor_key.clone(),
        participants,
        mode,
        status: AudioSessionStatus::Created,
        backend,
        capabilities,
        events: Vec::new(),
        next_seq: 0,
        expires_at_secs: expires_at,
    };
    state
        .audio_sessions
        .lock()
        .expect("audio sessions lock")
        .insert(session_id.clone(), audio_record.clone());

    state
        .webrtc
        .ensure_bridge(state.clone(), &audio_record)
        .await
        .map_err(|error| api_error(StatusCode::BAD_GATEWAY, &format!("WEBRTC_BRIDGE_CREATE_FAILED:{error}")))?;

    let mut response_headers = HeaderMap::new();
    if let Some(cookie) = actor.set_cookie {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response_headers.insert(SET_COOKIE, value);
        }
    }

    Ok((
        StatusCode::CREATED,
        response_headers,
        Json(json!({
            "status": "ok",
            "session": audio_session_metadata(&audio_record),
        })),
    ))
}

pub(crate) async fn get_audio_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let session = owned_audio_session(&state, &session_id, &site, &actor.actor_key)?;
    Ok(Json(json!({
        "status": "ok",
        "session": audio_session_metadata(&session),
    })))
}

pub(crate) async fn delete_audio_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);

    let session = {
        let mut sessions = state.audio_sessions.lock().expect("audio sessions lock");
        let Some(session) = sessions.remove(&session_id) else {
            return Err(api_error(StatusCode::NOT_FOUND, "AUDIO_SESSION_NOT_FOUND"));
        };
        if session.site != site {
            sessions.insert(session_id.clone(), session);
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "AUDIO_SESSION_SITE_MISMATCH",
            ));
        }
        if session.owner_key != actor.actor_key
            && !session
                .participants
                .iter()
                .any(|participant| participant == &actor.actor_key)
        {
            sessions.insert(session_id.clone(), session);
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "AUDIO_SESSION_ACCESS_DENIED",
            ));
        }
        session
    };

    state
        .rtc_sessions
        .lock()
        .expect("rtc sessions lock")
        .remove(&session.rtc_session_id);

    state.webrtc.close_session(&session.session_id).await;

    emit_server_event(
        &state,
        "AUDIO_SESSION_DELETED",
        json!({"audioSessionId": session_id, "site": site}),
    );

    Ok(Json(json!({ "status": "ok", "deleted": true })))
}

pub(crate) async fn post_audio_session_command(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<AudioSessionCommandRequest>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let session = owned_audio_session(&state, &session_id, &site, &actor.actor_key)?;
    let event_type = match body.command.as_str() {
        "start" => "audio.session.started",
        "stop" => "audio.session.stop_requested",
        "pause" => "audio.session.paused",
        "resume" => "audio.session.resumed",
        "interrupt" => "audio.session.interrupted",
        "mute_input" => "audio.session.input_muted",
        "unmute_input" => "audio.session.input_unmuted",
        _ => "audio.session.command",
    };
    let next_status = match body.command.as_str() {
        "start" => AudioSessionStatus::Streaming,
        "stop" => AudioSessionStatus::Paused,
        "pause" => AudioSessionStatus::Paused,
        "resume" => AudioSessionStatus::Streaming,
        _ => session.status.clone(),
    };
    let payload = body.payload.clone().unwrap_or_else(|| json!({}));

    let event = {
        let mut sessions = state.audio_sessions.lock().expect("audio sessions lock");
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "AUDIO_SESSION_NOT_FOUND"))?;
        session.status = next_status;
        append_audio_session_event(
            session,
            event_type.to_string(),
            json!({
                "audioSessionId": session_id,
                "rtcSessionId": session.rtc_session_id,
                "command": body.command,
                "payload": payload.clone(),
            }),
        )
    };
    broadcast_app_event(
        &state,
        audio_session_broadcast_event(&site, &session_id, &event),
    );

    state
        .webrtc
        .handle_command(&session_id, &body.command, payload)
        .await
        .map_err(|error| api_error(StatusCode::BAD_GATEWAY, &format!("WEBRTC_AUDIO_COMMAND_FAILED:{error}")))?;

    Ok(Json(json!({
        "status": "ok",
        "session": audio_session_metadata(&owned_audio_session(&state, &session_id, &site, &actor.actor_key)?),
        "backend": json!({"status":"ok"})
    })))
}
