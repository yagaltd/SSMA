//! Standardized server events and logging structure
//!
//! This module defines all server events with consistent naming and structure
//! for operator-facing logs and metrics.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Standardized event names for server events
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerEvent {
    // WebSocket events
    WsConnected,
    WsDisconnected,
    WsUnauthorizedFiltered,
    
    // SSE events
    SseConnected,
    SseDisconnected,
    SseClientDropped,
    SseUnauthorizedFiltered,
    
    // Channel events
    ChannelSubscribe,
    ChannelUnsubscribe,
    ChannelAccessDenied,
    ChannelResync,
    
    // Intent events
    IntentAcked,
    IntentRejected,
    IntentReworked,
    IntentUndone,
    
    // Auth events
    AuthLogin,
    AuthLogout,
    AuthRegister,
    
    // Media events
    MediaUploaded,
    MediaDeleted,
    
    // Backend events
    BackendEventsIngested,
    BackendAssetCreated,
    BackendAssetDeleted,
    
    // RTC events
    RtcSessionCreated,
    RtcSignalSent,
    
    // System events
    ShutdownInitiated,
    ShutdownComplete,
    RateLimitHit,
}

impl ServerEvent {
    /// Get the event name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerEvent::WsConnected => "WS_CONNECTED",
            ServerEvent::WsDisconnected => "WS_DISCONNECTED",
            ServerEvent::WsUnauthorizedFiltered => "WS_UNAUTHORIZED_FILTERED",
            ServerEvent::SseConnected => "SSE_CONNECTED",
            ServerEvent::SseDisconnected => "SSE_DISCONNECTED",
            ServerEvent::SseClientDropped => "SSE_CLIENT_DROPPED",
            ServerEvent::SseUnauthorizedFiltered => "SSE_UNAUTHORIZED_FILTERED",
            ServerEvent::ChannelSubscribe => "CHANNEL_SUBSCRIBE",
            ServerEvent::ChannelUnsubscribe => "CHANNEL_UNSUBSCRIBE",
            ServerEvent::ChannelAccessDenied => "CHANNEL_ACCESS_DENIED",
            ServerEvent::ChannelResync => "CHANNEL_RESYNC",
            ServerEvent::IntentAcked => "INTENT_ACKED",
            ServerEvent::IntentRejected => "INTENT_REJECTED",
            ServerEvent::IntentReworked => "INTENT_REWORKED",
            ServerEvent::IntentUndone => "INTENT_UNDONE",
            ServerEvent::AuthLogin => "AUTH_LOGIN",
            ServerEvent::AuthLogout => "AUTH_LOGOUT",
            ServerEvent::AuthRegister => "AUTH_REGISTER",
            ServerEvent::MediaUploaded => "MEDIA_UPLOADED",
            ServerEvent::MediaDeleted => "MEDIA_DELETED",
            ServerEvent::BackendEventsIngested => "BACKEND_EVENTS_INGESTED",
            ServerEvent::BackendAssetCreated => "BACKEND_ASSET_CREATED",
            ServerEvent::BackendAssetDeleted => "BACKEND_ASSET_DELETED",
            ServerEvent::RtcSessionCreated => "RTC_SESSION_CREATED",
            ServerEvent::RtcSignalSent => "RTC_SIGNAL_SENT",
            ServerEvent::ShutdownInitiated => "SHUTDOWN_INITIATED",
            ServerEvent::ShutdownComplete => "SHUTDOWN_COMPLETE",
            ServerEvent::RateLimitHit => "RATE_LIMIT_HIT",
        }
    }
}

/// Standardized log entry structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub timestamp: i64,
    pub level: LogLevel,
    pub event: String,
    pub message: String,
    pub context: Value,
    pub site: Option<String>,
    pub actor_key: Option<String>,
    pub connection_id: Option<String>,
}

/// Standardized log levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// Create a structured log entry
pub fn create_log_entry(
    level: LogLevel,
    event: ServerEvent,
    message: impl Into<String>,
    context: Value,
    site: Option<String>,
    actor_key: Option<String>,
    connection_id: Option<String>,
) -> LogEntry {
    LogEntry {
        timestamp: crate::runtime::now_millis(),
        level,
        event: event.as_str().to_string(),
        message: message.into(),
        context,
        site,
        actor_key,
        connection_id,
    }
}

/// Helper to create an event payload with consistent structure
pub fn event_payload(event: ServerEvent, data: Value) -> Value {
    json!({
        "event": event.as_str(),
        "timestamp": crate::runtime::now_millis(),
        "data": data,
    })
}
