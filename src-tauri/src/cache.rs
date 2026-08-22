use crate::models::{SnapshotStatus, UsageSnapshot};
use chrono::Utc;
use std::path::PathBuf;

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

pub fn load() -> Vec<UsageSnapshot> {
    let Ok(bytes) = std::fs::read(cache_path()) else {
        return Vec::new();
    };
    let Ok(mut snapshots) = serde_json::from_slice::<Vec<UsageSnapshot>>(&bytes) else {
        return Vec::new();
    };
    for snapshot in &mut snapshots {
        snapshot.status = SnapshotStatus::Stale;
        for window in &mut snapshot.windows {
            window.pace_limit_minutes = None;
        }
    }
    snapshots
}

pub fn save(snapshots: &[UsageSnapshot]) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(snapshots) {
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
            snapshot
        })
        .collect()
}
