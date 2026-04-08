//! File-based storage driver implementation
//!
//! This module provides the default file-based persistence using JSON files.

use crate::domain::persistence::{
    AppendOutcome, IntentRecord, IntentStorage, UserRecord, UserStorage,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedIntents {
    version: u8,
    entries: Vec<IntentRecord>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedUsers {
    version: u8,
    users: Vec<UserRecord>,
}

/// File-based intent storage
pub struct FileIntentStorage {
    path: PathBuf,
    state: Arc<Mutex<PersistedIntents>>,
    index: Arc<Mutex<HashMap<String, IntentRecord>>>,
    replay_window_ms: u64,
    max_entries: usize,
}

impl FileIntentStorage {
    pub fn new(path: PathBuf, replay_window_ms: u64, max_entries: usize) -> Self {
        let persisted = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<PersistedIntents>(&raw).ok())
                .unwrap_or_default()
        } else {
            PersistedIntents::default()
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
    
    fn dedupe_key(id: &str, site: &str) -> String {
        format!("{}:{}", site, id)
    }
    
    fn trim_replay_window_locked(&self, state: &mut PersistedIntents) {
        let now = crate::runtime::now_millis();
        let replay_start = now.saturating_sub(self.replay_window_ms as i64);
        state.entries.retain(|e| e.inserted_at >= replay_start);
    }
    
    fn rebuild_index_locked(&self, state: &PersistedIntents, index: &mut HashMap<String, IntentRecord>) {
        index.clear();
        for entry in &state.entries {
            index.insert(Self::dedupe_key(&entry.id, &entry.site), entry.clone());
        }
    }
    
    fn flush_locked(&self, state: &PersistedIntents) -> Result<(), String> {
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("Failed to serialize intent store: {}", e))?;
        
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        
        use std::io::Write;
        file.write_all(raw.as_bytes())
            .map_err(|e| format!("Failed to write intent store: {}", e))?;
        
        file.sync_all()
            .map_err(|e| format!("Failed to sync intent store: {}", e))?;
        
        drop(file);
        
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Failed to rename intent store: {}", e))?;
        
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        
        Ok(())
    }
}

impl IntentStorage for FileIntentStorage {
    fn append_batch(&self, mut entries: Vec<IntentRecord>) -> Result<AppendOutcome, String> {
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
        self.flush_locked(&state)?;
        
        Ok(AppendOutcome { all, fresh, replayed })
    }

    fn get(&self, id: &str, site: &str) -> Option<IntentRecord> {
        let index = self.index.lock().ok()?;
        index.get(&Self::dedupe_key(id, site)).cloned()
    }

    fn update_status(&self, id: &str, site: &str, status: &str) -> Option<IntentRecord> {
        self.update_entry(id, site, |entry| {
            entry.status = status.to_string();
        })
    }

    fn update_entry<F>(&self, id: &str, site: &str, updater: F) -> Option<IntentRecord>
    where
        F: FnOnce(&mut IntentRecord),
    {
        let key = Self::dedupe_key(id, site);
        let mut state = self.state.lock().ok()?;
        let mut index = self.index.lock().ok()?;
        let mut updated = None;
        
        if let Some(entry) = index.get_mut(&key) {
            updater(entry);
            updated = Some(entry.clone());
            
            // Update in state
            if let Some(pos) = state.entries.iter_mut().find(|e| Self::dedupe_key(&e.id, &e.site) == key) {
                *pos = entry.clone();
            }
            
            let _ = self.flush_locked(&state);
        }
        
        updated
    }

    fn entries_after(&self, cursor: u64, limit: usize) -> Vec<IntentRecord> {
        let state = self.state.lock().expect("store state lock");
        state.entries
            .iter()
            .filter(|e| e.log_seq > cursor)
            .take(limit)
            .cloned()
            .collect()
    }

    fn latest_cursor(&self) -> u64 {
        let state = self.state.lock().expect("store state lock");
        state.entries.iter().map(|e| e.log_seq).max().unwrap_or(0)
    }

    fn total_entries(&self) -> usize {
        let state = self.state.lock().expect("store state lock");
        state.entries.len()
    }

    fn flush(&self) -> Result<(), String> {
        let state = self.state.lock().expect("store state lock");
        self.flush_locked(&state)
    }
}

/// File-based user storage
pub struct FileUserStorage {
    path: PathBuf,
    state: Arc<Mutex<PersistedUsers>>,
}

impl FileUserStorage {
    pub fn new(path: PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        
        let persisted = if path.exists() {
            let data = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str::<PersistedUsers>(&data).unwrap_or_default()
        } else {
            PersistedUsers::default()
        };
        
        Self {
            path,
            state: Arc::new(Mutex::new(persisted)),
        }
    }
    
    fn persist(&self) -> Result<(), String> {
        let data = self.state.lock().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?;
        
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        
        Ok(())
    }
}

impl UserStorage for FileUserStorage {
    fn find_by_email(&self, email: &str) -> Option<UserRecord> {
        let data = self.state.lock().ok()?;
        data.users.iter().find(|u| u.email == email).cloned()
    }

    fn find_by_id(&self, id: &str) -> Option<UserRecord> {
        let data = self.state.lock().ok()?;
        data.users.iter().find(|u| u.id == id).cloned()
    }

    fn create(&self, record: UserRecord) -> Result<UserRecord, String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            if data.users.iter().any(|u| u.email == record.email) {
                return Err("EMAIL_TAKEN".to_string());
            }
            data.users.push(record.clone());
        }
        self.persist()?;
        Ok(record)
    }

    fn update_login(&self, id: &str) -> Result<(), String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.id == id) {
                user.last_login_at = Some(now);
                user.updated_at = now;
            }
        }
        self.persist()
    }
}
