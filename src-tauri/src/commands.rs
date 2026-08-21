use crate::models::*;
use crate::providers;
use serde::{Deserialize, Serialize};
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
    fn default() -> Self { Self { enabled: serde_json::json!({"claude":true,"codex":true,"grok":true,"cursor":true,"opencode":true}), refresh_seconds: 60, launch_at_login: false, theme: "dark".into() } }
}

pub struct AppState { pub snapshots: Arc<RwLock<Vec<UsageSnapshot>>> }

#[tauri::command]
pub async fn get_snapshots(state: State<'_, AppState>) -> Result<Vec<UsageSnapshot>, String> {
    let current = state.snapshots.read().await;
    if current.is_empty() { drop(current); let fresh = providers::mock_snapshots().await; *state.snapshots.write().await = fresh.clone(); return Ok(fresh); }
    Ok(current.clone())
}

#[tauri::command]
pub async fn refresh_snapshots(state: State<'_, AppState>) -> Result<Vec<UsageSnapshot>, String> {
    let fresh = providers::mock_snapshots().await;
    *state.snapshots.write().await = fresh.clone();
    Ok(fresh)
}

#[tauri::command]
pub fn get_settings() -> AppSettings { AppSettings::default() }

#[tauri::command]
pub fn save_profile(_provider: ProviderId, _name: String) -> Result<(), String> { Ok(()) }

#[tauri::command]
pub fn delete_profile(_provider: ProviderId, _name: String) -> Result<(), String> { Ok(()) }
