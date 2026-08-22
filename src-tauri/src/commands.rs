use crate::cache;
use crate::live;
use crate::models::*;
use crate::profiles;
use crate::providers;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;
use crate::settings;

pub struct AppState {
    pub snapshots: Arc<RwLock<Vec<UsageSnapshot>>>,
    pub settings: Arc<RwLock<AppSettings>>,
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
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    eprintln!("settings: toggle received");
    settings::save(&settings)?;
    *state.settings.write().await = settings;
    Ok(())
}

#[tauri::command]
pub fn save_profile(provider: ProviderId, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 48
        || name.eq_ignore_ascii_case("personal")
        || name.eq_ignore_ascii_case("all")
    {
        return Err("Profile name is invalid".into());
    }
    profiles::save(&provider, name)
}

#[tauri::command]
pub fn delete_profile(provider: ProviderId, name: String) -> Result<(), String> {
    profiles::delete(&provider, name.trim())
}

#[tauri::command]
pub fn list_profiles(provider: ProviderId) -> Vec<String> {
    profiles::list(&provider)
}

#[tauri::command]
pub fn save_manual_credential(provider: ProviderId, value: String) -> Result<(), String> {
    profiles::save_manual(&provider, &value)
}
