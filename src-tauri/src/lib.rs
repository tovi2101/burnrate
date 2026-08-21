mod backoff;
mod cache;
mod commands;
mod live;
mod models;
mod providers;

use commands::AppState;
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cached = cache::load();
    let lock_path = std::env::temp_dir().join("burnrate-single-instance.lock");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path);
    if lock.is_err() {
        return;
    }
    tauri::Builder::default()
        .manage(AppState {
            snapshots: Arc::new(RwLock::new(cached)),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh_snapshots,
            commands::get_settings,
            commands::save_profile,
            commands::delete_profile,
            commands::list_profiles
        ])
        .setup(|app| {
            let menu = tauri::menu::MenuBuilder::new(app)
                .text("open", "Open Burnrate")
                .separator()
                .text("quit", "Quit")
                .build()?;
            if let Some(icon) = app.default_window_icon().cloned() {
                tauri::tray::TrayIconBuilder::with_id("burnrate")
                    .icon(icon)
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "open" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|app, event| {
                        if let tauri::tray::TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            ..
                        } = event
                        {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }
            if app.get_webview_window("main").is_none() {
                let _ =
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                        .build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!(), move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_file(&lock_path);
            }
        })
        .expect("error while running Burnrate");
}
