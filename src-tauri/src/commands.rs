use crate::cache;
use crate::live;
use crate::models::*;
use crate::providers;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub enabled: serde_json::Value,
    pub refresh_seconds: u64,
    pub launch_at_login: bool,
    pub theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled: serde_json::json!({"claude":true,"codex":true,"grok":true,"cursor":true,"opencode":true}),
            refresh_seconds: 60,
            launch_at_login: false,
            theme: "dark".into(),
        }
    }
}

pub struct AppState {
    pub snapshots: Arc<RwLock<Vec<UsageSnapshot>>>,
}

#[tauri::command]
pub async fn get_snapshots(state: State<'_, AppState>) -> Result<Vec<UsageSnapshot>, String> {
    let current = state.snapshots.read().await;
    if current.is_empty() {
        drop(current);
        let fresh = live::fetch_live().await;
        let fresh = if fresh.is_empty() {
            providers::mock_snapshots().await
        } else {
            fresh
        };
        cache::save(&fresh);
        *state.snapshots.write().await = fresh.clone();
        return Ok(fresh);
    }
    Ok(current.clone())
}

#[tauri::command]
pub async fn refresh_snapshots(state: State<'_, AppState>) -> Result<Vec<UsageSnapshot>, String> {
    let fresh = live::fetch_live().await;
    let fresh = if fresh.is_empty() {
        let current = state.snapshots.read().await.clone();
        if current.is_empty() {
            providers::mock_snapshots().await
        } else {
            cache::stale(&current)
        }
    } else {
        cache::save(&fresh);
        fresh
    };
    *state.snapshots.write().await = fresh.clone();
    Ok(fresh)
}

#[tauri::command]
pub fn get_settings() -> AppSettings {
    AppSettings::default()
}

#[tauri::command]
pub fn save_profile(provider: ProviderId, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 48 {
        return Err("Profile name is invalid".into());
    }
    let source =
        credential_path(&provider).ok_or_else(|| "No CLI credential file found".to_string())?;
    let contents = std::fs::read_to_string(source)
        .map_err(|_| "Credential file could not be read".to_string())?;
    let service = "dev.burnrate.app";
    let account = format!("profile:{}:{}", provider_key(&provider), name);
    Entry::new(service, &account)
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&contents)
        .map_err(|_| "OS keyring write failed".to_string())
}

#[tauri::command]
pub fn delete_profile(provider: ProviderId, name: String) -> Result<(), String> {
    let account = format!("profile:{}:{}", provider_key(&provider), name.trim());
    Entry::new("dev.burnrate.app", &account)
        .map_err(|_| "OS keyring unavailable".to_string())?
        .delete_credential()
        .map_err(|_| "OS keyring delete failed".to_string())
}

#[tauri::command]
pub fn list_profiles(provider: ProviderId) -> Vec<String> {
    // Keyring backends do not provide portable account enumeration. The current CLI login is
    // always available as Personal; saved names are mirrored by the frontend until next launch.
    let _ = provider;
    vec!["Personal".into()]
}

fn provider_key(provider: &ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
        ProviderId::Cursor => "cursor",
        ProviderId::Opencode => "opencode",
    }
}

fn credential_path(provider: &ProviderId) -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(PathBuf::from)?
    } else {
        std::env::var_os("HOME").map(PathBuf::from)?
    };
    let path = match provider {
        ProviderId::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"))
            .join(".credentials.json"),
        ProviderId::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("auth.json"),
        ProviderId::Grok => std::env::var_os("GROK_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".grok"))
            .join("auth.json"),
        ProviderId::Cursor | ProviderId::Opencode => return None,
    };
    path.exists().then_some(path)
}
