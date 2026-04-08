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
    pub actor_key: Option<String>,
    pub user_id: Option<String>,
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
        self.rebuild_index_locked(&state, &mut index);
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

    pub fn update_entry<F>(&self, id: &str, site: &str, updater: F) -> Option<IntentRecord>
    where
        F: Fn(&mut IntentRecord),
    {
        let key = Self::dedupe_key(id, site);
        let mut state = self.state.lock().expect("store state lock");
        let mut index = self.index.lock().expect("store index lock");
        let mut updated = None;

        if let Some(entry) = index.get_mut(&key) {
            updater(entry);
            updated = Some(entry.clone());
        }

        for entry in state.entries.iter_mut() {
            if entry.id == id && entry.site == site {
                updater(entry);
                updated = Some(entry.clone());
            }
        }

        if updated.is_some() {
            let _ = self.flush_locked(&state);
        }

        updated
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

    fn rebuild_index_locked(
        &self,
        state: &PersistedStore,
        index: &mut HashMap<String, IntentRecord>,
    ) {
        index.clear();
        for entry in &state.entries {
            index.insert(Self::dedupe_key(&entry.id, &entry.site), entry.clone());
        }
    }

    pub fn flush_to_disk(&self) -> std::io::Result<()> {
        let state = self.state.lock().expect("store state lock");
        self.flush_locked(&state)
    }

    fn flush_locked(&self, state: &PersistedStore) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(state)
            .unwrap_or_else(|_| "{\"version\":1,\"entries\":[]}".to_string());
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp)?;
        use std::io::Write;
        file.write_all(raw.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &self.path)?;
        // Sync parent directory for crash durability
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_intent(id: &str, site: &str) -> IntentRecord {
        IntentRecord {
            id: id.to_string(),
            intent: "TEST".to_string(),
            payload: serde_json::json!({}),
            meta: serde_json::json!({"clock": 1}),
            inserted_at: now_millis(),
            log_seq: 0,
            site: site.to_string(),
            status: "acked".to_string(),
            connection_id: None,
            actor_key: None,
            user_id: None,
            backend: None,
        }
    }

    #[test]
    fn store_creates_empty_file_on_new() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.json");
        let store = IntentStore::new(path.clone(), 300_000, 100);
        assert_eq!(store.total_entries(), 0);
    }

    #[test]
    fn store_persists_to_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.json");
        let store = IntentStore::new(path.clone(), 300_000, 100);
        store.append_batch(vec![make_intent("a", "s1")]);
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("a"));
    }

    #[test]
    fn atomic_write_creates_no_tmp_file_after_flush() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.json");
        let store = IntentStore::new(path.clone(), 300_000, 100);
        store.append_batch(vec![make_intent("x", "s1")]);
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn deduplication_uses_site_and_id() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.json");
        let store = IntentStore::new(path.clone(), 300_000, 100);

        let r1 = make_intent("dup", "site-a");
        let r2 = make_intent("dup", "site-a");
        let r3 = make_intent("dup", "site-b");

        let first = store.append_batch(vec![r1]);
        let second = store.append_batch(vec![r2]);
        let third = store.append_batch(vec![r3]);

        assert_eq!(first.fresh.len(), 1);
        assert_eq!(second.fresh.len(), 0); // same site+id → deduped
        assert_eq!(second.replayed.len(), 1);
        assert_eq!(third.fresh.len(), 1);   // different site → fresh
    }
}
