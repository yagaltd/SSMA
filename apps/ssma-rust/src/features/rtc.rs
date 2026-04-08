use crate::transport::{
    api_error, broadcast_app_event, purge_expired_runtime_state, request_site,
    resolve_actor_from_headers, ApiResult, AppState, CreateRtcSessionRequest,
    PostRtcSignalRequest, RtcSessionRecord, RtcSignalRecord,
};
use crate::runtime::now_millis;
use axum::extract::{Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn rtc_channel_name(session_id: &str) -> String {
    format!("rtc.session.{}", session_id)
}

fn rtc_signal_event_intent(session_id: &str, signal: &RtcSignalRecord, channel: &str) -> Value {
    json!({
        "id": format!("rtc:{}:{}", session_id, signal.seq),
        "intent": "RTC_SIGNAL",
        "payload": {
            "sessionId": session_id,
            "kind": signal.kind,
            "senderId": signal.sender_id,
            "targetId": signal.target_id,
            "payload": signal.payload,
            "createdAt": signal.created_at_ms,
            "seq": signal.seq,
        },
        "meta": {
            "channels": [channel],
            "ephemeral": true,
        },
        "insertedAt": signal.created_at_ms,
        "logSeq": signal.seq,
    })
}

pub(crate) fn rtc_snapshot_for_channel(
    state: &Arc<AppState>,
    channel: &str,
    cursor: u64,
) -> Option<(Vec<Value>, u64)> {
    let session_id = channel.strip_prefix("rtc.session.")?;
    let sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
    let session = sessions.get(session_id)?;
    let intents = session
        .signals
        .iter()
        .filter(|signal| signal.seq > cursor)
        .map(|signal| rtc_signal_event_intent(&session.session_id, signal, channel))
        .collect::<Vec<_>>();
    let next = session
        .signals
        .last()
        .map(|signal| signal.seq)
        .unwrap_or(cursor);
    Some((intents, next))
}

pub fn emit_rtc_signal(
    state: &Arc<AppState>,
    site: &str,
    session_id: &str,
    kind: &str,
    sender_id: &str,
    target_id: Option<String>,
    payload: Value,
) -> ApiResult<u64> {
    let channel = rtc_channel_name(session_id);
    let signal = {
        let mut sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "RTC_SESSION_NOT_FOUND"))?;
        session.next_seq += 1;
        let signal = RtcSignalRecord {
            seq: session.next_seq,
            kind: kind.to_string(),
            sender_id: sender_id.to_string(),
            target_id,
            payload,
            created_at_ms: now_millis(),
        };
        session.signals.push(signal.clone());
        if session.signals.len() > 64 {
            let overflow = session.signals.len() - 64;
            session.signals.drain(0..overflow);
        }
        signal
    };
    broadcast_app_event(
        state,
        json!({
            "type": "invalidate",
            "reason": "rtc-signal",
            "site": site,
            "cursor": signal.seq,
            "intents": [rtc_signal_event_intent(session_id, &signal, &channel)],
        }),
    );
    Ok(signal.seq)
}

pub(crate) async fn create_rtc_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateRtcSessionRequest>,
) -> ApiResult<impl IntoResponse> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, true)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    let session_id = Uuid::new_v4().to_string();
    let mut participants = vec![actor.actor_key.clone()];
    for participant in body.participants.unwrap_or_default() {
        if !participants.iter().any(|value| value == &participant) {
            participants.push(participant);
        }
    }
    let now = crate::runtime::now_secs();
    let ttl = body.ttl_secs.unwrap_or(state.config.media_ttl_secs);
    let record = RtcSessionRecord {
        session_id: session_id.clone(),
        site: site.clone(),
        owner_key: actor.actor_key.clone(),
        participants,
        signals: Vec::new(),
        next_seq: 0,
        expires_at_secs: now + ttl,
    };
    state
        .rtc_sessions
        .lock()
        .expect("rtc sessions lock")
        .insert(session_id.clone(), record);
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
            "session": {
                "sessionId": session_id.clone(),
                "channel": rtc_channel_name(&session_id),
                "site": site,
                "owner": actor.actor_id,
                "createdAt": now,
                "expiresAt": now + ttl,
            }
        })),
    ))
}

pub(crate) async fn post_rtc_signal(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<PostRtcSignalRequest>,
) -> ApiResult<Json<Value>> {
    purge_expired_runtime_state(&state);
    let actor = resolve_actor_from_headers(&headers, &state.config, false)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
    let site = request_site(&headers);
    {
        let sessions = state.rtc_sessions.lock().expect("rtc sessions lock");
        let Some(session) = sessions.get(&session_id) else {
            return Err(api_error(StatusCode::NOT_FOUND, "RTC_SESSION_NOT_FOUND"));
        };
        if session.site != site {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "RTC_SESSION_SITE_MISMATCH",
            ));
        }
        if session.owner_key != actor.actor_key
            && !session
                .participants
                .iter()
                .any(|participant| participant == &actor.actor_key)
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "RTC_SESSION_ACCESS_DENIED",
            ));
        }
    }

    let seq = emit_rtc_signal(
        &state,
        &site,
        &session_id,
        &body.kind,
        &body.sender_id,
        body.target_id.clone(),
        body.payload.clone(),
    )?;
    Ok(Json(json!({
        "status": "ok",
        "sessionId": session_id,
        "seq": seq,
    })))
}
