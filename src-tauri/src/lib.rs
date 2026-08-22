mod backoff;
mod cache;
mod commands;
mod live;
mod models;
mod profiles;
mod providers;
mod settings;

use commands::{AppState, TrayRegistration};
use std::sync::Arc;
use tauri::{Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::RwLock;

#[cfg(debug_assertions)]
macro_rules! invoke_handler {
    () => {
        tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh_snapshots,
            commands::get_settings,
            commands::save_settings,
            commands::save_profile,
            commands::delete_profile,
            commands::list_profiles,
            commands::save_manual_credential,
            commands::debug_tray_state,
            commands::debug_simulate_tray_click
        ]
    };
}

#[cfg(not(debug_assertions))]
macro_rules! invoke_handler {
    () => {
        tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh_snapshots,
            commands::get_settings,
            commands::save_settings,
            commands::save_profile,
            commands::delete_profile,
            commands::list_profiles,
            commands::save_manual_credential
        ]
    };
}

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

fn tray_anchor(app: &tauri::AppHandle) -> Option<(i32, i32)> {
    if let Some(tray) = app.tray_by_id("burnrate") {
        if let Some(rect) = tray.rect().ok().flatten() {
            if let Ok(value) = serde_json::to_value(rect) {
                let position = value.get("position");
                let size = value.get("size");
                let number = |value: Option<&serde_json::Value>| {
                    value
                        .and_then(serde_json::Value::as_i64)
                        .or_else(|| value.and_then(serde_json::Value::as_u64).map(|n| n as i64))
                        .or_else(|| value.and_then(serde_json::Value::as_f64).map(|n| n as i64))
                };
                if let (Some(x), Some(y), Some(height)) = (
                    number(position.and_then(|value| value.get("x"))),
                    number(position.and_then(|value| value.get("y"))),
                    number(size.and_then(|value| value.get("height"))),
                ) {
                    return Some((x as i32, y as i32 + height as i32 + 8));
                }
            }
        }
    }
    // Windows can return no tray rectangle for a notification-area icon. The
    // click still arrives with the cursor over that icon, so this fallback
    // keeps the popover anchored immediately above the same point.
    #[cfg(windows)]
    {
        let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) } != 0 {
            return Some((point.x - 190, point.y - 580));
        }
        let width = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(0) };
        let height = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(1) };
        #[cfg(debug_assertions)]
        eprintln!("tray: windows metrics width={width} height={height}");
        if width > 0 && height > 0 {
            return Some((width - 380, height - 580));
        }
    }
    let cursor = app.cursor_position().ok()?;
    Some((cursor.x as i32 - 190, cursor.y as i32 - 580))
}

/// Shared by the real tray click callback and the development IPC proof command.
pub fn show_popover(app: &tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let positioned_near_tray = if let Some((x, y)) = tray_anchor(app) {
        match window.set_position(PhysicalPosition::new(x, y)) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("tray: set position failed: {error}");
                false
            }
        }
    } else {
        eprintln!("tray: no anchor position available");
        false
    };
    let _ = window.show();
    let _ = window.set_focus();
    positioned_near_tray
}

#[cfg(debug_assertions)]
fn write_tray_icon_preview(icon: &tauri::image::Image<'_>) -> Result<(), String> {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("screenshots")
        .join("verified-tray-icon.png");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let scale = 4_u32;
    let width = icon.width() * scale;
    let height = icon.height() * scale;
    let file = std::fs::File::create(output).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    let source = icon.rgba();
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let source_x = x / scale;
            let source_y = y / scale;
            let index = ((source_y * icon.width() + source_x) * 4) as usize;
            pixels.extend_from_slice(&source[index..index + 4]);
        }
    }
    writer.write_image_data(&pixels).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cached = cache::load();
    let loaded_settings = settings::load();
    let lock_path = std::env::temp_dir().join("burnrate-single-instance.lock");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path);
    let Ok(lock) = lock else { return };
    let lock_file = Arc::new(std::sync::Mutex::new(Some(lock)));
    let cleanup_lock = Arc::clone(&lock_file);
    tauri::Builder::default()
        .manage(AppState {
            snapshots: Arc::new(RwLock::new(cached)),
            settings: Arc::new(RwLock::new(loaded_settings)),
            tray: Arc::new(std::sync::Mutex::new(TrayRegistration::default())),
        })
        .invoke_handler(invoke_handler!())
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("lifecycle")
                .on_event(move |_app, event| {
                    if matches!(event, tauri::RunEvent::Exit) {
                        if let Ok(mut file) = cleanup_lock.lock() {
                            file.take();
                        }
                        let _ = std::fs::remove_file(&lock_path);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let menu = tauri::menu::MenuBuilder::new(app)
                .text("open", "Open Burnrate")
                .separator()
                .text("quit", "Quit")
                .build()?;
            let icon = app
                .default_window_icon()
                .map(|icon| icon.clone().to_owned())
                .unwrap_or_else(tray_icon);
            let icon_width = icon.width();
            let icon_height = icon.height();
            #[cfg(debug_assertions)]
            let preview_icon = icon.clone();
            tauri::tray::TrayIconBuilder::with_id("burnrate")
                .icon(icon)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(window) = app.app_handle().get_webview_window("main") {
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
                        show_popover(&app.app_handle());
                    }
                })
                .build(app)?;
            let app_state = app.state::<AppState>();
            if let Ok(mut registration) = app_state.tray.lock() {
                registration.registered = true;
                registration.icon_width = icon_width;
                registration.icon_height = icon_height;
            }
            #[cfg(debug_assertions)]
            write_tray_icon_preview(&preview_icon).map_err(std::io::Error::other)?;
            if app.get_webview_window("main").is_none() {
                let _ =
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                        .build()?;
            }
            #[cfg(debug_assertions)]
            if std::env::var_os("BURNRATE_SHOW_WINDOW").is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Burnrate");
}
