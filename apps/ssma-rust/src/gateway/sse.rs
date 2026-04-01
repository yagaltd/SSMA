use super::{
    emit_server_event, extract_event_channels, is_island_authorized, resolve_user_from_headers,
    AppState, SseQuery,
};
use async_stream::stream;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub(crate) fn parse_requested_islands(raw: Option<&str>) -> Option<Vec<String>> {
    let islands = raw
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    if islands.is_empty() {
        None
    } else {
        Some(islands)
    }
}

pub(crate) fn is_sse_event_authorized(
    state: &Arc<AppState>,
    role: &str,
    requested_islands: Option<&Vec<String>>,
    event: &Value,
) -> bool {
    if extract_event_channels(event)
        .iter()
        .any(|channel| channel.starts_with("rtc.session.") || channel.starts_with("audio.session."))
    {
        return false;
    }
    is_island_authorized(state, role, requested_islands, event)
}

pub(crate) async fn sse_events(
    Query(query): Query<SseQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let site = query.site.clone().unwrap_or_else(|| "default".to_string());
    let replay_cursor = query.cursor.unwrap_or(0);
    let islands = parse_requested_islands(query.islands.as_deref());
    let user = resolve_user_from_headers(&headers, &state.config);
    let auth_role = user
        .as_ref()
        .map(|resolved| resolved.role.clone())
        .unwrap_or_else(|| "guest".to_string());
    let replay = state.store.entries_after(replay_cursor, 500);
    let replay_cursor = replay
        .last()
        .map(|entry| entry.log_seq)
        .unwrap_or(replay_cursor);

    state.metrics.sse_total.fetch_add(1, Ordering::Relaxed);
    state.metrics.sse_active.fetch_add(1, Ordering::Relaxed);

    let mut rx = state.events.subscribe();
    let state_for_stream = state.clone();
    let stream = stream! {
        yield Ok(Event::default().event("ready").data(json!({ "service": "ssma-rust" }).to_string()));
        yield Ok(Event::default().event("replay").data(json!({ "intents": replay, "cursor": replay_cursor }).to_string()));

        loop {
            match rx.recv().await {
                Ok(event) => {
                    let event_site = event.get("site").and_then(|v| v.as_str()).unwrap_or("default");
                    if event_site != site {
                        continue;
                    }
                    if !is_sse_event_authorized(&state_for_stream, &auth_role, islands.as_ref(), &event) {
                        state_for_stream
                            .metrics
                            .sse_unauthorized_filtered
                            .fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("message").to_string();
                    yield Ok(Event::default().event(event_type).data(event.to_string()));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    state_for_stream.metrics.sse_client_dropped.fetch_add(1, Ordering::Relaxed);
                    emit_server_event(&state_for_stream, "SSE_CLIENT_DROPPED", json!({"site": site, "skipped": skipped}));
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }

        state_for_stream.metrics.sse_active.fetch_sub(1, Ordering::Relaxed);
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(20))
            .text("keepalive"),
    )
}
