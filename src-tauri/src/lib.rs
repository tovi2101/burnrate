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

fn tray_icon() -> tauri::image::Image<'static> {
    let mut rgba = vec![0_u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            let index = (y * 16 + x) * 4;
            let inside = (2..14).contains(&x) && (2..14).contains(&y);
            rgba[index] = if inside { 25 } else { 0 };
            rgba[index + 1] = if inside { 24 } else { 0 };
            rgba[index + 2] = if inside { 28 } else { 0 };
            rgba[index + 3] = if inside { 255 } else { 0 };
        }
    }
    for (x, height, color) in [
        (5, 5, [143, 203, 155]),
        (8, 8, [230, 170, 84]),
        (11, 10, [242, 109, 120]),
    ] {
        for y in (14 - height)..14 {
            let index = (y * 16 + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    tauri::image::Image::new_owned(rgba, 16, 16)
}

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
            let icon = app.default_window_icon().cloned().unwrap_or_else(tray_icon);
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
