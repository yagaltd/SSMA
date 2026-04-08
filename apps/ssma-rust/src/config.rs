use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub subprotocol: String,
    pub backend_url: String,
    pub backend_internal_token: String,
    pub auth_cookie_name: String,
    pub anonymous_cookie_name: String,
    pub auth_jwt_secret: String,
    pub require_auth_for_writes: bool,
    pub replay_window_ms: u64,
    pub intent_store_path: PathBuf,
    pub media_storage_root: PathBuf,
    pub media_max_upload_bytes: u64,
    pub media_ttl_secs: u64,
    pub global_rate_window_ms: i64,
    pub global_rate_max: u32,
    pub channel_subscribe_window_ms: i64,
    pub channel_subscribe_max: u32,
    pub protected_channels: Vec<String>,
    pub protected_channel_min_role: String,
    pub island_access: HashMap<String, String>,
    pub user_store_path: PathBuf,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub access_ttl_ms: u64,
    pub auth_cookie_secure: bool,
    pub auth_cookie_same_site: String,
    pub allowed_origins: String,
    pub optimistic_rework_window_ms: u64,
    pub optimistic_rework_max: u32,
    pub sse_retry_ms: u64,
    pub ws_max_buffered_bytes: u64,
    pub optimistic_max_entries: usize,
    pub log_relay_url: String,
    pub backend_timeout_ms: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let canonical = std::env::var("SSMA_PROTOCOL_SUBPROTOCOL").ok();
        let legacy = std::env::var("SSMA_OPTIMISTIC_SUBPROTOCOL").ok();
        let subprotocol = canonical.or(legacy).unwrap_or_else(|| "1.0.0".to_string());

        let host = std::env::var("SSMA_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("SSMA_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(5050);
        let backend_url = std::env::var("SSMA_BACKEND_URL").unwrap_or_default();
        let backend_internal_token =
            std::env::var("SSMA_BACKEND_INTERNAL_TOKEN").unwrap_or_default();
        let auth_cookie_name =
            std::env::var("SSMA_AUTH_COOKIE").unwrap_or_else(|_| "ssma_session".to_string());
        let anonymous_cookie_name =
            std::env::var("SSMA_ANON_COOKIE").unwrap_or_else(|_| "ssma_anon".to_string());
        let auth_jwt_secret = std::env::var("SSMA_AUTH_JWT_SECRET")
            .unwrap_or_else(|_| {
                tracing::warn!("SSMA_AUTH_JWT_SECRET not set – using insecure default. This MUST NOT be used in production!");
                "change-me-in-production".to_string()
            });
        let require_auth_for_writes = std::env::var("SSMA_OPTIMISTIC_REQUIRE_AUTH_WRITES")
            .map(|v| v == "true")
            .unwrap_or(false);
        let replay_window_ms = std::env::var("SSMA_OPTIMISTIC_REPLAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5 * 60 * 1000);

        let intent_store_path = std::env::var("SSMA_OPTIMISTIC_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data/optimistic-intents-rust.json"));
        let media_storage_root = std::env::var("SSMA_MEDIA_STORAGE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data/media"));
        let media_max_upload_bytes = std::env::var("SSMA_MEDIA_MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(50 * 1024 * 1024);
        let media_ttl_secs = std::env::var("SSMA_MEDIA_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60 * 60);
        let global_rate_window_ms = std::env::var("SSMA_RATE_WINDOW_MS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(60_000);
        let global_rate_max = std::env::var("SSMA_RATE_MAX")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(120);
        let channel_subscribe_window_ms = std::env::var("SSMA_OPTIMISTIC_CHANNEL_WINDOW_MS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(10_000);
        let channel_subscribe_max = std::env::var("SSMA_OPTIMISTIC_CHANNEL_MAX")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(8);
        let protected_channels = std::env::var("SSMA_OPTIMISTIC_PROTECTED_CHANNELS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(|part| part.trim())
                    .filter(|part| !part.is_empty())
                    .map(|part| part.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let protected_channel_min_role =
            std::env::var("SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE")
                .unwrap_or_else(|_| "admin".to_string());

        let user_store_path = std::env::var("SSMA_USER_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data/users.json"));
        let jwt_issuer = std::env::var("SSMA_JWT_ISSUER")
            .unwrap_or_else(|_| "ssma-auth-service".to_string());
        let jwt_audience = std::env::var("SSMA_JWT_AUDIENCE")
            .unwrap_or_else(|_| "csma-clients".to_string());
        let access_ttl_ms = std::env::var("SSMA_ACCESS_TTL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(900_000);
        let auth_cookie_secure = std::env::var("SSMA_AUTH_COOKIE_SECURE")
            .map(|v| v == "true")
            .unwrap_or(true);
        let auth_cookie_same_site = std::env::var("SSMA_AUTH_COOKIE_SAMESITE")
            .unwrap_or_else(|_| "Lax".to_string());
        let allowed_origins =
            std::env::var("SSMA_ALLOWED_ORIGINS").unwrap_or_default();
        let optimistic_rework_window_ms = std::env::var("SSMA_OPTIMISTIC_REWORK_WINDOW_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60_000);
        let optimistic_rework_max = std::env::var("SSMA_OPTIMISTIC_REWORK_MAX")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(20);
        let sse_retry_ms = std::env::var("SSMA_SSE_RETRY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2500);
        let ws_max_buffered_bytes = std::env::var("SSMA_WS_MAX_BUFFERED_BYTES")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(262144);
        let optimistic_max_entries = std::env::var("SSMA_OPTIMISTIC_MAX_ENTRIES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(5000);
        let log_relay_url = std::env::var("SSMA_LOG_RELAY_URL").unwrap_or_default();
        let backend_timeout_ms = std::env::var("SSMA_BACKEND_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5000);

        Self {
            host,
            port,
            subprotocol,
            backend_url,
            backend_internal_token,
            auth_cookie_name,
            anonymous_cookie_name,
            auth_jwt_secret,
            require_auth_for_writes,
            replay_window_ms,
            intent_store_path,
            media_storage_root,
            media_max_upload_bytes,
            media_ttl_secs,
            global_rate_window_ms,
            global_rate_max,
            channel_subscribe_window_ms,
            channel_subscribe_max,
            protected_channels,
            protected_channel_min_role,
            island_access: default_island_access(),
            user_store_path,
            jwt_issuer,
            jwt_audience,
            access_ttl_ms,
            auth_cookie_secure,
            auth_cookie_same_site,
            allowed_origins,
            optimistic_rework_window_ms,
            optimistic_rework_max,
            sse_retry_ms,
            ws_max_buffered_bytes,
            optimistic_max_entries,
            log_relay_url,
            backend_timeout_ms,
        }
    }

    /// Validate configuration for production readiness.
    /// Returns Ok(()) if valid, or Err with description of the first failure.
    pub fn validate(&self) -> Result<(), String> {
        // Validate required secrets in production
        if self.auth_jwt_secret == "change-me-in-production" {
            return Err("SSMA_AUTH_JWT_SECRET must be set in production".to_string());
        }

        // Validate path writability
        if let Some(parent) = self.intent_store_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create intent store directory: {}", e))?;
        }
        // Try to create/open the intent store file
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.intent_store_path)
            .map_err(|e| format!("Cannot write to intent store path: {}", e))?;

        if let Some(parent) = self.user_store_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create user store directory: {}", e))?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.user_store_path)
            .map_err(|e| format!("Cannot write to user store path: {}", e))?;

        if let Some(parent) = self.media_storage_root.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create media storage directory: {}", e))?;
        }
        std::fs::create_dir_all(&self.media_storage_root)
            .map_err(|e| format!("Cannot create media storage directory: {}", e))?;

        // Validate allowed origins format
        if !self.allowed_origins.is_empty() && self.allowed_origins != "*" {
            let origins: Vec<&str> = self.allowed_origins.split(',').collect();
            for origin in origins {
                let origin = origin.trim();
                if !origin.is_empty() {
                    // Basic URL validation
                    if !origin.starts_with("http://") && !origin.starts_with("https://") {
                        return Err(format!(
                            "Invalid origin '{}': must start with http:// or https://",
                            origin
                        ));
                    }
                }
            }
        }

        // Validate cookie configuration consistency
        if self.auth_cookie_secure && !self.allowed_origins.is_empty() && self.allowed_origins != "*" {
            // If secure cookies are enabled, ensure HTTPS is being used
            // This is a warning-level check, not a hard error
            tracing::warn!(
                "auth_cookie_secure=true with explicit origins - ensure reverse proxy handles HTTPS"
            );
        }

        Ok(())
    }
}

fn default_island_access() -> HashMap<String, String> {
    HashMap::from([
        ("product-inventory".to_string(), "guest".to_string()),
        ("product-reviews".to_string(), "user".to_string()),
        ("blog-comments".to_string(), "user".to_string()),
        ("hydration-test".to_string(), "guest".to_string()),
        ("ops.dashboard".to_string(), "staff".to_string()),
    ])
}
