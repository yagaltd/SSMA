pub use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    max_entries: usize,
}

impl IntentStore {
    pub fn new(path: PathBuf, replay_window_ms: u64, max_entries: usize) -> Self {
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
            max_entries,
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
        state
            .entries
            .retain(|entry| entry.inserted_at >= replay_start);
        while state.entries.len() > self.max_entries {
            state.entries.remove(0);
        }
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

pub fn now_secs() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs()
}
