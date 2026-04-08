//! Persistence layer with driver abstraction
//!
//! This module defines a trait-based abstraction for persistence drivers,
//! allowing SSMA to use either file-based (JSON) or SQLite storage.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Intent record stored in persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentRecord {
    pub id: String,
    pub intent: String,
    pub payload: Value,
    pub meta: Value,
    pub inserted_at: i64,
    pub log_seq: u64,
    pub site: String,
    pub status: String,
    pub connection_id: Option<String>,
    pub actor_key: Option<String>,
    pub user_id: Option<String>,
    pub backend: Option<Value>,
}

/// User record stored in persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_login_at: Option<i64>,
}

/// Trait for intent storage drivers
pub trait IntentStorage: Send + Sync {
    /// Append batch of intent records
    fn append_batch(&self, entries: Vec<IntentRecord>) -> Result<AppendOutcome, String>;
    
    /// Get intent by ID and site
    fn get(&self, id: &str, site: &str) -> Option<IntentRecord>;
    
    /// Update intent status
    fn update_status(&self, id: &str, site: &str, status: &str) -> Option<IntentRecord>;
    
    /// Update intent with custom function
    fn update_entry<F>(&self, id: &str, site: &str, updater: F) -> Option<IntentRecord>
    where
        F: FnOnce(&mut IntentRecord);
    
    /// Get entries after cursor
    fn entries_after(&self, cursor: u64, limit: usize) -> Vec<IntentRecord>;
    
    /// Get latest cursor
    fn latest_cursor(&self) -> u64;
    
    /// Get total entries count
    fn total_entries(&self) -> usize;
    
    /// Flush to disk (for file-based drivers)
    fn flush(&self) -> Result<(), String>;
}

/// Trait for user storage drivers
pub trait UserStorage: Send + Sync {
    /// Find user by email
    fn find_by_email(&self, email: &str) -> Option<UserRecord>;
    
    /// Find user by ID
    fn find_by_id(&self, id: &str) -> Option<UserRecord>;
    
    /// Create new user
    fn create(&self, record: UserRecord) -> Result<UserRecord, String>;
    
    /// Update user last login time
    fn update_login(&self, id: &str) -> Result<(), String>;
}

/// Append operation outcome
#[derive(Debug, Clone)]
pub struct AppendOutcome {
    pub all: Vec<IntentRecord>,
    pub fresh: Vec<IntentRecord>,
    pub replayed: Vec<IntentRecord>,
}

/// Storage driver type selector
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum StorageDriver {
    File,
    Sqlite,
}

impl StorageDriver {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "file" => Some(StorageDriver::File),
            "sqlite" => Some(StorageDriver::Sqlite),
            _ => None,
        }
    }
    
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageDriver::File => "file",
            StorageDriver::Sqlite => "sqlite",
        }
    }
}

/// Configuration for storage drivers
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub driver: StorageDriver,
    pub intent_store_path: PathBuf,
    pub user_store_path: PathBuf,
    pub replay_window_ms: u64,
    pub max_entries: usize,
}

impl StorageConfig {
    pub fn from_env() -> Self {
        let driver_str = std::env::var("SSMA_STORAGE_DRIVER")
            .unwrap_or_else(|_| "file".to_string());
        let driver = StorageDriver::from_str(&driver_str)
            .unwrap_or(StorageDriver::File);
        
        StorageConfig {
            driver,
            intent_store_path: std::env::var("SSMA_OPTIMISTIC_STORE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./data/optimistic-intents-rust.json")),
            user_store_path: std::env::var("SSMA_USER_STORE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./data/users.json")),
            replay_window_ms: std::env::var("SSMA_OPTIMISTIC_REPLAY_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5 * 60 * 1000),
            max_entries: std::env::var("SSMA_OPTIMISTIC_MAX_ENTRIES")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(5000),
        }
    }
}
