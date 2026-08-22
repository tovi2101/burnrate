use crate::cache;
use crate::live;
use crate::models::*;
use crate::profiles;
use crate::providers;
use serde::Serialize;
#[cfg(debug_assertions)]
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(debug_assertions)]
use tauri::Manager;
use tauri::State;
use tokio::sync::RwLock;
use crate::settings;

pub struct AppState {
    pub snapshots: Arc<RwLock<Vec<UsageSnapshot>>>,
    pub settings: Arc<RwLock<AppSettings>>,
    pub tray: Arc<Mutex<TrayRegistration>>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TrayRegistration {
    pub registered: bool,
    pub icon_width: u32,
    pub icon_height: u32,
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

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize)]
pub struct DebugTrayState {
    pub registered: bool,
    pub icon_width: u32,
    pub icon_height: u32,
    pub bars: BTreeMap<String, Vec<f64>>,
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_tray_state(
    state: State<'_, AppState>,
) -> Result<DebugTrayState, String> {
    let registration = state
        .tray
        .lock()
        .map_err(|_| "tray state lock poisoned".to_string())?
        .clone();
    let snapshots = state.snapshots.read().await;
    let mut bars = BTreeMap::new();
    for snapshot in snapshots.iter() {
        bars.insert(
            format!("{}:{}", snapshot.provider.to_string(), snapshot.profile_name),
            snapshot.windows.iter().map(|window| window.used_pct).collect(),
        );
    }
    let result = DebugTrayState {
        registered: registration.registered,
        icon_width: registration.icon_width,
        icon_height: registration.icon_height,
        bars,
    };
    eprintln!("debug_tray_state: {result:?}");
    Ok(result)
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize)]
pub struct DebugTrayClick {
    pub visible: bool,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub positioned_near_tray: bool,
}

#[cfg(debug_assertions)]
#[tauri::command]
pub fn debug_simulate_tray_click(app: tauri::AppHandle) -> Result<DebugTrayClick, String> {
    let positioned_near_tray = crate::show_popover(&app);
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is unavailable".to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let position = window
        .outer_position()
        .map_err(|error| error.to_string())?;
    let result = DebugTrayClick {
        visible: window.is_visible().unwrap_or(false),
        width: size.width,
        height: size.height,
        x: position.x,
        y: position.y,
        positioned_near_tray,
    };
    eprintln!("debug_simulate_tray_click: {result:?}");
    Ok(result)
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
