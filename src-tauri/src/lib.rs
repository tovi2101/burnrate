mod commands;
mod models;
mod providers;

use commands::AppState;
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState { snapshots: Arc::new(RwLock::new(Vec::new())) })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh_snapshots,
            commands::get_settings,
            commands::save_profile,
            commands::delete_profile
        ])
        .setup(|app| {
            let menu = tauri::menu::MenuBuilder::new(app)
                .text("open", "Open Burnrate")
                .separator()
                .text("quit", "Quit")
                .build()?;
            let icon = app.default_window_icon().cloned().ok_or("missing default icon")?;
            tauri::tray::TrayIconBuilder::with_id("burnrate")
                .icon(icon)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => { if let Some(window) = app.get_webview_window("main") { let _ = window.show(); let _ = window.set_focus(); } }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|app, event| {
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, .. } = event {
                        if let Some(window) = app.get_webview_window("main") { let _ = window.show(); let _ = window.set_focus(); }
                    }
                })
                .build(app)?;
            if app.get_webview_window("main").is_none() {
                let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into())).build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Burnrate");
}
