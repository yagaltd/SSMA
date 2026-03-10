use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub subprotocol: String,
    pub backend_url: String,
    pub backend_internal_token: String,
    pub auth_cookie_name: String,
    pub auth_jwt_secret: String,
    pub require_auth_for_writes: bool,
    pub replay_window_ms: u64,
    pub intent_store_path: PathBuf,
    pub global_rate_window_ms: i64,
    pub global_rate_max: u32,
    pub channel_subscribe_window_ms: i64,
    pub channel_subscribe_max: u32,
    pub protected_channels: Vec<String>,
    pub protected_channel_min_role: String,
    pub island_access: HashMap<String, String>,
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
        let backend_internal_token = std::env::var("SSMA_BACKEND_INTERNAL_TOKEN").unwrap_or_default();
        let auth_cookie_name = std::env::var("SSMA_AUTH_COOKIE").unwrap_or_else(|_| "ssma_session".to_string());
        let auth_jwt_secret =
            std::env::var("SSMA_AUTH_JWT_SECRET").unwrap_or_else(|_| "change-me-in-production".to_string());
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
            std::env::var("SSMA_OPTIMISTIC_PROTECTED_CHANNEL_MIN_ROLE").unwrap_or_else(|_| "admin".to_string());

        Self {
            host,
            port,
            subprotocol,
            backend_url,
            backend_internal_token,
            auth_cookie_name,
            auth_jwt_secret,
            require_auth_for_writes,
            replay_window_ms,
            intent_store_path,
            global_rate_window_ms,
            global_rate_max,
            channel_subscribe_window_ms,
            channel_subscribe_max,
            protected_channels,
            protected_channel_min_role,
            island_access: default_island_access(),
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentRecord {
    pub id: String,
    pub intent: String,
    pub payload: serde_json::Value,
    pub meta: serde_json::Value,
    pub inserted_at: i64,
    pub log_seq: u64,
    pub site: String,
    pub status: String,
    pub connection_id: Option<String>,
    pub backend: Option<serde_json::Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedStore {
    version: u8,
    entries: Vec<IntentRecord>,
}

#[derive(Debug, Clone)]
pub struct AppendOutcome {
    pub all: Vec<IntentRecord>,
    pub fresh: Vec<IntentRecord>,
    pub replayed: Vec<IntentRecord>,
}

#[derive(Debug, Clone)]
pub struct IntentStore {
    path: PathBuf,
    state: Arc<Mutex<PersistedStore>>,
    index: Arc<Mutex<HashMap<String, IntentRecord>>>,
    replay_window_ms: u64,
}

impl IntentStore {
    pub fn new(path: PathBuf, replay_window_ms: u64) -> Self {
        let persisted = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<PersistedStore>(&raw).ok())
                .unwrap_or_else(|| PersistedStore {
                    version: 1,
                    entries: vec![],
                })
        } else {
            PersistedStore {
                version: 1,
                entries: vec![],
            }
        };
        let mut index = HashMap::new();
        for entry in &persisted.entries {
            index.insert(Self::dedupe_key(&entry.id, &entry.site), entry.clone());
        }
        Self {
            path,
            state: Arc::new(Mutex::new(persisted)),
            index: Arc::new(Mutex::new(index)),
            replay_window_ms,
        }
    }

    pub fn append_batch(&self, mut entries: Vec<IntentRecord>) -> AppendOutcome {
        let mut state = self.state.lock().expect("store state lock");
        let mut index = self.index.lock().expect("store index lock");
        let mut max_seq = state.entries.iter().map(|e| e.log_seq).max().unwrap_or(0);

        let mut all = Vec::new();
        let mut fresh = Vec::new();
        let mut replayed = Vec::new();
        for entry in entries.iter_mut() {
            let key = Self::dedupe_key(&entry.id, &entry.site);
            if let Some(existing) = index.get(&key) {
                all.push(existing.clone());
                replayed.push(existing.clone());
                continue;
            }
            max_seq += 1;
            entry.log_seq = max_seq;
            index.insert(key, entry.clone());
            state.entries.push(entry.clone());
            all.push(entry.clone());
            fresh.push(entry.clone());
        }

        self.trim_replay_window_locked(&mut state);
        let _ = self.flush_locked(&state);

        AppendOutcome {
            all,
            fresh,
            replayed,
        }
    }

    pub fn get(&self, id: &str, site: &str) -> Option<IntentRecord> {
        let index = self.index.lock().expect("store index lock");
        index.get(&Self::dedupe_key(id, site)).cloned()
    }

    pub fn update_status(
        &self,
        id: &str,
        site: &str,
        status: &str,
        backend: Option<serde_json::Value>,
    ) {
        let key = Self::dedupe_key(id, site);
        let mut state = self.state.lock().expect("store state lock");
        let mut index = self.index.lock().expect("store index lock");

        if let Some(entry) = index.get_mut(&key) {
            entry.status = status.to_string();
            entry.backend = backend.clone();
        }

        for entry in state.entries.iter_mut() {
            if entry.id == id && entry.site == site {
                entry.status = status.to_string();
                entry.backend = backend.clone();
            }
        }
        let _ = self.flush_locked(&state);
    }

    pub fn entries_after(&self, cursor: u64, limit: usize) -> Vec<IntentRecord> {
        let state = self.state.lock().expect("store state lock");
        state
            .entries
            .iter()
            .filter(|entry| entry.log_seq > cursor)
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn latest_cursor(&self) -> u64 {
        let state = self.state.lock().expect("store state lock");
        state.entries.last().map(|entry| entry.log_seq).unwrap_or(0)
    }

    pub fn total_entries(&self) -> usize {
        let state = self.state.lock().expect("store state lock");
        state.entries.len()
    }

    fn trim_replay_window_locked(&self, state: &mut PersistedStore) {
        let now = now_millis();
        let replay_start = now.saturating_sub(self.replay_window_ms as i64);
        state.entries.retain(|entry| entry.inserted_at >= replay_start);
    }

    fn flush_locked(&self, state: &PersistedStore) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(state)
            .unwrap_or_else(|_| "{\"version\":1,\"entries\":[]}".to_string());
        fs::write(&self.path, raw)
    }

    fn dedupe_key(id: &str, site: &str) -> String {
        format!("{}::{}", site, id)
    }
}

pub fn now_millis() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
