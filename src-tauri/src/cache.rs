use crate::models::{SnapshotStatus, UsageSnapshot};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct CachedState {
    pub snapshots: Vec<UsageSnapshot>,
    pub notified: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheDocument {
    version: u8,
    snapshots: Vec<UsageSnapshot>,
    #[serde(default)]
    notified: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredCache {
    Document(CacheDocument),
    Legacy(Vec<UsageSnapshot>),
}

fn cache_path() -> PathBuf {
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(|p| PathBuf::from(p).join(".cache"))
                    .unwrap_or_else(|| PathBuf::from("."))
            })
    };
    root.join("burnrate").join("usage-cache.json")
}

pub fn load() -> CachedState {
    let Ok(bytes) = std::fs::read(cache_path()) else {
        return CachedState::default();
    };
    let Ok(stored) = serde_json::from_slice::<StoredCache>(&bytes) else {
        return CachedState::default();
    };
    let (mut snapshots, notified) = match stored {
        StoredCache::Document(document) => (document.snapshots, document.notified),
        StoredCache::Legacy(snapshots) => (snapshots, BTreeMap::new()),
    };
    for snapshot in &mut snapshots {
        snapshot.status = SnapshotStatus::Stale;
        for window in &mut snapshot.windows {
            window.pace_limit_minutes = None;
        }
    }
    CachedState {
        snapshots,
        notified,
    }
}

pub fn save(snapshots: &[UsageSnapshot], notified: &BTreeMap<String, Vec<u8>>) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let document = CacheDocument {
        version: 1,
        snapshots: snapshots.to_vec(),
        notified: notified.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec(&document) {
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

pub fn stale(snapshots: &[UsageSnapshot]) -> Vec<UsageSnapshot> {
    snapshots
        .iter()
        .cloned()
        .map(|mut snapshot| {
            snapshot.status = SnapshotStatus::Stale;
            snapshot.error_message = None;
            snapshot.fetched_at = snapshot.fetched_at.min(Utc::now());
            for window in &mut snapshot.windows {
                window.pace_limit_minutes = None;
            }
            snapshot
        })
        .collect()
}
