use super::{
    broadcast_app_event, can_access_channel, consume_channel_rate_limit, consume_global_rate_limit,
    emit_server_event, extract_event_channels, normalize_status,
    register_channel_subscription, resolve_actor_from_headers, store_entries_for_channel_after,
    subprotocol_major_match, teardown_connection_state, unregister_channel_subscription,
    AppState, ConnectionContext, WsQuery,
};
use crate::backend::{BackendContext, BackendUser};
use crate::runtime::IntentRecord;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use uuid::Uuid;

pub(crate) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, query, headers, state))
}

async fn ws_session(socket: WebSocket, query: WsQuery, headers: HeaderMap, state: Arc<AppState>) {
    state.metrics.ws_total.fetch_add(1, Ordering::Relaxed);
    state.metrics.ws_active.fetch_add(1, Ordering::Relaxed);

    let transport_role = query.role.unwrap_or_else(|| "follower".to_string());
    let site = query.site.unwrap_or_else(|| "default".to_string());
    let connection_id = Uuid::new_v4().to_string();
    let actor = resolve_actor_from_headers(&headers, &state.config, false);
    let ip = super::connection_ip_from_headers(&headers);
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let user_id = actor.as_ref().and_then(|resolved| resolved.user_id.clone());
    let auth_role = actor
        .as_ref()
        .map(|resolved| resolved.role.clone())
        .unwrap_or_else(|| "guest".to_string());
    let context = ConnectionContext {
        transport_role: transport_role.clone(),
        auth_role: auth_role.clone(),
        site: site.clone(),
        connection_id: connection_id.clone(),
        actor_key: actor.as_ref().map(|value| value.actor_key.clone()),
        user_id,
        ip,
        user_agent,
    };

    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.events.subscribe();
    let max_pending_messages = (state.config.ws_max_buffered_bytes / 1024).max(1) as usize;

    let client_subprotocol = query
        .subprotocol
        .clone()
        .unwrap_or_else(|| state.config.subprotocol.clone());
    if !subprotocol_major_match(&state.config.subprotocol, &client_subprotocol) {
        let _ = sender
            .send(Message::Text(
                json!({
                    "type": "error",
                    "code": "SUBPROTOCOL_MISMATCH",
                    "expected": state.config.subprotocol
                })
                .to_string(),
            ))
            .await;
        let _ = sender.send(Message::Close(None)).await;
        teardown_connection_state(&state, &connection_id);
        return;
    }

    let hello = json!({
        "type": "hello",
        "role": transport_role,
        "authRole": auth_role,
        "subprotocol": state.config.subprotocol,
        "connectionId": connection_id,
        "serverTime": crate::runtime::now_millis(),
    });
    let _ = sender.send(Message::Text(hello.to_string())).await;

    let cursor = query.cursor.unwrap_or(0);
    let replay = state.store.entries_after(cursor, 500);
    let replay_cursor = replay.last().map(|entry| entry.log_seq).unwrap_or(cursor);
    let _ = sender
        .send(Message::Text(
            json!({ "type": "replay", "intents": replay, "cursor": replay_cursor }).to_string(),
        ))
        .await;

    loop {
        tokio::select! {
            maybe_msg = receiver.next() => {
                let Some(Ok(message)) = maybe_msg else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };

                let global_key = format!("ws:{}", context.connection_id);
                let globally_allowed = consume_global_rate_limit(
                    &state,
                    global_key,
                    state.config.global_rate_max,
                    state.config.global_rate_window_ms,
                );
                if !globally_allowed {
                    state.metrics.rate_limit_hits.fetch_add(1, Ordering::Relaxed);
                    let _ = sender
                        .send(Message::Text(
                            json!({ "type": "error", "code": "RATE_LIMITED", "retryAfterMs": state.config.global_rate_window_ms }).to_string(),
                        ))
                        .await;
                    continue;
                }

                let payload = match serde_json::from_str::<Value>(&text) {
                    Ok(v) => v,
                    Err(_) => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "error", "code": "INVALID_JSON" }).to_string()))
                            .await;
                        continue;
                    }
                };

                if let Err(details) = crate::protocol::validate_inbound(&payload) {
                    let _ = sender
                        .send(Message::Text(
                            json!({ "type": "error", "code": "INVALID_CONTRACT", "details": details }).to_string(),
                        ))
                        .await;
                    continue;
                }

                let msg_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                match msg_type {
                    "ping" => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "pong", "ts": crate::runtime::now_millis() }).to_string()))
                            .await;
                    }
                    "intent.batch" => {
                        handle_intent_batch(&mut sender, &state, &context, payload).await;
                    }
                    "channel.subscribe" => {
                        handle_channel_subscribe(&mut sender, &state, &context, payload).await;
                    }
                    "channel.unsubscribe" => {
                        handle_channel_unsubscribe(&mut sender, &state, &context, payload).await;
                    }
                    "channel.resync" => {
                        handle_channel_resync(&mut sender, &state, &context, payload).await;
                    }
                    "channel.command" => {
                        handle_channel_command(&mut sender, &state, &context, payload).await;
                    }
                    _ => {
                        let _ = sender
                            .send(Message::Text(json!({ "type": "error", "code": "UNKNOWN_TYPE" }).to_string()))
                            .await;
                    }
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(event) => {
                        for frame in build_frames_for_connection(&state, &context, &event) {
                            let _ = sender.send(Message::Text(frame.to_string())).await;
                        }
                        // Backpressure check: count how many messages are buffered
                        let mut pending = 0;
                        while let Ok(event) = event_rx.try_recv() {
                            pending += build_frames_for_connection(&state, &context, &event).len();
                            if pending > max_pending_messages {
                                break;
                            }
                        }
                        if pending > max_pending_messages {
                            let _ = sender.send(Message::Text(
                                json!({ "type": "error", "code": "BACKPRESSURE_CLOSE" }).to_string(),
                            )).await;
                            let _ = sender.send(Message::Close(Some(axum::extract::ws::CloseFrame {
                                code: 1008u16,
                                reason: "backpressure".into(),
                            }))).await;
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    teardown_connection_state(&state, &connection_id);
}

async fn handle_intent_batch(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    if context.transport_role != "leader" {
        let _ = sender
            .send(Message::Text(
                json!({ "type": "error", "code": "NOT_LEADER" }).to_string(),
            ))
            .await;
        return;
    }
    if state.config.require_auth_for_writes && context.user_id.is_none() {
        let _ = sender
            .send(Message::Text(
                json!({ "type": "error", "code": "UNAUTHORIZED" }).to_string(),
            ))
            .await;
        emit_server_event(
            state,
            "INTENT_REJECTED",
            json!({"reason":"UNAUTHORIZED", "site": context.site}),
        );
        return;
    }

    let intents = payload
        .get("intents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let now = crate::runtime::now_millis();
    let records = intents
        .iter()
        .map(|intent| IntentRecord {
            id: intent
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            intent: intent
                .get("intent")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string(),
            payload: intent.get("payload").cloned().unwrap_or(Value::Null),
            meta: intent.get("meta").cloned().unwrap_or_else(|| json!({})),
            inserted_at: now,
            log_seq: 0,
            site: context.site.clone(),
            status: "acked".to_string(),
            connection_id: Some(context.connection_id.clone()),
            backend: None,
        })
        .collect::<Vec<_>>();

    let outcome = state.store.append_batch(records);
    for row in &outcome.all {
        emit_server_event(
            state,
            "INTENT_ACCEPTED",
            json!({"id": row.id, "site": row.site, "logSeq": row.log_seq}),
        );
    }

    let mut status_by_id = HashMap::<String, String>::new();
    for replayed in &outcome.replayed {
        status_by_id.insert(replayed.id.clone(), replayed.status.clone());
    }
    for fresh in &outcome.fresh {
        status_by_id.insert(fresh.id.clone(), fresh.status.clone());
    }

    if !outcome.fresh.is_empty() {
        emit_server_event(
            state,
            "INTENT_FORWARDED",
            json!({"site": context.site, "count": outcome.fresh.len()}),
        );
        let backend_ctx = BackendContext {
            site: context.site.clone(),
            actor_key: context.actor_key.clone(),
            connection_id: Some(context.connection_id.clone()),
            ip: Some(context.ip.clone()),
            user_agent: context.user_agent.clone(),
            user: Some(BackendUser {
                id: context.user_id.clone(),
                role: context.auth_role.clone(),
            }),
        };
        let fresh_payload = outcome
            .fresh
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "intent": entry.intent,
                    "payload": entry.payload,
                    "meta": entry.meta,
                    "logSeq": entry.log_seq,
                    "insertedAt": entry.inserted_at,
                })
            })
            .collect::<Vec<_>>();

        match state
            .backend
            .apply_intents(fresh_payload, &backend_ctx)
            .await
        {
            Ok(result) => {
                if let Some(results) = result.get("results").and_then(|v| v.as_array()) {
                    for row in results {
                        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        let status = normalize_status(row.get("status").and_then(|v| v.as_str()));
                        status_by_id.insert(id.to_string(), status.to_string());
                        state.store.update_status(
                            id,
                            &context.site,
                            status,
                            row.get("code").cloned(),
                        );
                        if let Some(events) = row.get("events").and_then(|v| v.as_array()) {
                            for event in events {
                                super::internal::publish_backend_event(state, event);
                            }
                        }
                    }
                }
                if let Some(events) = result.get("events").and_then(|v| v.as_array()) {
                    for event in events {
                        super::internal::publish_backend_event(state, event);
                    }
                }
            }
            Err(error) => {
                for row in &outcome.fresh {
                    status_by_id.insert(row.id.clone(), "failed".to_string());
                    state.store.update_status(
                        &row.id,
                        &row.site,
                        "failed",
                        Some(json!({ "message": error.to_string() })),
                    );
                }
            }
        }
    }

    let ack_intents = intents
        .iter()
        .map(|intent| {
            let id = intent.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            let persisted = state.store.get(id, &context.site);
            let status = status_by_id.get(id).cloned().unwrap_or_else(|| "acked".to_string());
            if status == "acked" {
                emit_server_event(state, "INTENT_ACKED", json!({"id": id, "site": context.site}));
            } else {
                emit_server_event(state, "INTENT_REJECTED", json!({"id": id, "site": context.site, "status": status}));
            }
            json!({
                "id": id,
                "status": status,
                "serverTimestamp": persisted.as_ref().map(|e| e.inserted_at).unwrap_or_else(crate::runtime::now_millis),
                "site": context.site,
                "logSeq": persisted.as_ref().map(|e| e.log_seq).unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();

    let _ = sender
        .send(Message::Text(
            json!({ "type": "ack", "intents": ack_intents }).to_string(),
        ))
        .await;

    let invalidate_intents = outcome
        .all
        .iter()
        .filter(|entry| {
            status_by_id
                .get(&entry.id)
                .map(|s| s == "acked")
                .unwrap_or(true)
        })
        .map(|entry| {
            json!({
                "id": entry.id,
                "intent": entry.intent,
                "payload": entry.payload,
                "meta": entry.meta,
                "insertedAt": entry.inserted_at,
                "logSeq": entry.log_seq,
            })
        })
        .collect::<Vec<_>>();

    if !invalidate_intents.is_empty() {
        broadcast_app_event(
            state,
            json!({
                "type": "invalidate",
                "reason": "intent-flush",
                "site": context.site,
                "cursor": state.store.latest_cursor(),
                "intents": invalidate_intents,
            }),
        );
    }
}

async fn handle_channel_subscribe(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));

    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.ack", "status": "error", "channel": channel, "params": params.clone(), "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params.clone(), "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }

    let key = format!("{}:{}:{}", context.site, context.connection_id, channel);
    let rate_allowed = consume_channel_rate_limit(
        state,
        key,
        state.config.channel_subscribe_max,
        state.config.channel_subscribe_window_ms,
    );
    if !rate_allowed {
        state
            .metrics
            .rate_limit_hits
            .fetch_add(1, Ordering::Relaxed);
        let _ = sender
            .send(Message::Text(
                json!({
                    "type": "channel.ack",
                    "status": "error",
                    "channel": channel,
                    "code": "RATE_LIMITED",
                    "retryAfterMs": state.config.channel_subscribe_window_ms,
                })
                .to_string(),
            ))
            .await;
        return;
    }

    register_channel_subscription(
        state,
        &context.connection_id,
        &context.site,
        channel,
        params.clone(),
    );

    emit_server_event(
        state,
        "CHANNEL_SUBSCRIBE",
        json!({"channel": channel, "site": context.site, "connectionId": context.connection_id}),
    );

    let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.ack", "status": "ok", "channel": channel, "params": params.clone() }).to_string(),
            ))
            .await;

    let mut snapshot_cursor = state.store.latest_cursor();
    let intents =
        if let Some((rtc_intents, rtc_cursor)) = super::rtc::rtc_snapshot_for_channel(state, channel, 0) {
            snapshot_cursor = rtc_cursor;
            rtc_intents
        } else if let Some((audio_intents, audio_cursor)) =
            super::audio::audio_snapshot_for_channel(state, channel, 0)
        {
            snapshot_cursor = audio_cursor;
            audio_intents
        } else if state.backend.is_configured() {
            let backend_ctx = BackendContext {
                site: context.site.clone(),
                actor_key: context.actor_key.clone(),
                connection_id: Some(context.connection_id.clone()),
                ip: Some(context.ip.clone()),
                user_agent: context.user_agent.clone(),
                user: Some(BackendUser {
                    id: context.user_id.clone(),
                    role: context.auth_role.clone(),
                }),
            };
            match state
                .backend
                .subscribe(channel, params.clone(), &backend_ctx)
                .await
            {
                Ok(response) if response.get("status").and_then(|v| v.as_str()) == Some("ok") => {
                    snapshot_cursor = response
                        .get("cursor")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(snapshot_cursor);
                    response
                        .get("snapshot")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                }
                _ => store_entries_for_channel_after(state, channel, 0, 200)
                    .into_iter()
                    .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                    .collect(),
            }
        } else {
            store_entries_for_channel_after(state, channel, 0, 200)
                .into_iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                .collect()
        };

    let _ = sender
        .send(Message::Text(
            json!({
                "type": "channel.snapshot",
                "channel": channel,
                "params": params.clone(),
                "intents": intents,
                "cursor": snapshot_cursor,
            })
            .to_string(),
        ))
        .await;
}

async fn handle_channel_unsubscribe(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    unregister_channel_subscription(state, &context.connection_id, channel, &params);

    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.unsubscribed", "status": "ok", "channel": channel })
                .to_string(),
        ))
        .await;
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.close", "status": "ok", "channel": channel, "params": params, "reason": "client-unsubscribe" }).to_string(),
        ))
        .await;
}

async fn handle_channel_resync(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params, "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }
    let cursor = payload.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0);
    let (intents, next) =
        if let Some((rtc_intents, rtc_cursor)) = super::rtc::rtc_snapshot_for_channel(state, channel, cursor) {
            (rtc_intents, rtc_cursor)
        } else if let Some((audio_intents, audio_cursor)) =
            super::audio::audio_snapshot_for_channel(state, channel, cursor)
        {
            (audio_intents, audio_cursor)
        } else {
            let intents = store_entries_for_channel_after(state, channel, cursor, 200);
            let next = intents.last().map(|entry| entry.log_seq).unwrap_or(cursor);
            (
                intents
                    .into_iter()
                    .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                    .collect::<Vec<_>>(),
                next,
            )
        };
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.replay", "status": "ok", "channel": channel, "params": params, "cursor": next, "intents": intents }).to_string(),
        ))
        .await;
}

async fn handle_channel_command(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
    context: &ConnectionContext,
    payload: Value,
) {
    let channel = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("global");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    if !can_access_channel(state, channel, context) {
        emit_server_event(
            state,
            "CHANNEL_ACCESS_DENIED",
            json!({"channel": channel, "site": context.site, "role": context.auth_role}),
        );
        let _ = sender
            .send(Message::Text(
                json!({ "type": "channel.close", "status": "error", "channel": channel, "params": params, "code": "ACCESS_DENIED" }).to_string(),
            ))
            .await;
        return;
    }
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let _ = sender
        .send(Message::Text(
            json!({ "type": "channel.command", "status": "ok", "channel": channel, "params": params, "command": command }).to_string(),
        ))
        .await;
}

fn build_frames_for_connection(
    state: &Arc<AppState>,
    context: &ConnectionContext,
    event: &Value,
) -> Vec<Value> {
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("message");
    if event_type == "island.invalidate" {
        if super::is_island_authorized(state, &context.auth_role, None, event) {
            return vec![event.clone()];
        }
        state
            .metrics
            .ws_unauthorized_filtered
            .fetch_add(1, Ordering::Relaxed);
        return Vec::new();
    }

    if event_type != "invalidate" {
        return Vec::new();
    }

    let row = {
        let registry = state
            .channel_registry
            .lock()
            .expect("channel registry lock");
        match registry.get(&context.connection_id) {
            Some(row) => row.clone(),
            None => return Vec::new(),
        }
    };

    let event_site = event
        .get("site")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if event_site != row.site {
        return Vec::new();
    }

    let event_channels = extract_event_channels(event);
    if event_channels.is_empty() {
        return Vec::new();
    }

    let intents = event.get("intents").cloned().unwrap_or_else(|| json!([]));
    let reason = event
        .get("reason")
        .cloned()
        .unwrap_or_else(|| json!("backend-event"));
    let cursor = event
        .get("cursor")
        .cloned()
        .unwrap_or_else(|| json!(state.store.latest_cursor()));
    let site = row.site.clone();

    row.subscriptions
        .values()
        .filter(|subscription| {
            event_channels
                .iter()
                .any(|channel_id| channel_id == &subscription.channel)
        })
        .map(|subscription| {
            json!({
                "type": "channel.invalidate",
                "site": site.clone(),
                "channel": subscription.channel.clone(),
                "params": subscription.params.clone(),
                "reason": reason.clone(),
                "cursor": cursor.clone(),
                "intents": intents.clone(),
            })
        })
        .collect()
}
