mod backoff;
mod cache;
mod commands;
mod history;
pub mod live;
pub mod models;
mod pace;
pub mod profiles;
mod providers;
mod settings;
mod warnings;

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
            commands::get_history,
            commands::save_settings,
            commands::save_profile,
            commands::delete_profile,
            commands::list_profiles,
            commands::get_account_setup,
            commands::begin_add_account,
            commands::detect_new_account,
            commands::cancel_add_account,
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
            commands::get_history,
            commands::save_settings,
            commands::save_profile,
            commands::delete_profile,
            commands::list_profiles,
            commands::get_account_setup,
            commands::begin_add_account,
            commands::detect_new_account,
            commands::cancel_add_account,
            commands::save_manual_credential
        ]
    };
}

fn tray_icon() -> tauri::image::Image<'static> {
    let mut rgba = vec![0_u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let index = (y * 32 + x) * 4;
            let border = (2..30).contains(&x) && (2..30).contains(&y);
            let inside = (4..28).contains(&x) && (4..28).contains(&y);
            rgba[index..index + 4].copy_from_slice(if inside {
                &[16, 17, 21, 255]
            } else if border {
                &[242, 244, 248, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    for (x, height, color) in [
        (8, 7, [220, 139, 102]),
        (12, 11, [143, 203, 155]),
        (16, 15, [190, 150, 237]),
        (20, 9, [102, 202, 209]),
        (24, 13, [242, 109, 120]),
    ] {
        for y in (25 - height)..25 {
            let index = (y * 32 + x) * 4;
            rgba[index..index + 8].copy_from_slice(&[
                color[0], color[1], color[2], 255, color[0], color[1], color[2], 255,
            ]);
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

#[cfg(target_os = "windows")]
fn platform_tray_icon() -> (tauri::image::Image<'static>, String) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("icons")
        .join("icon.ico");
    match tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico")) {
        Ok(icon) => (icon.to_owned(), path.display().to_string()),
        Err(error) => {
            eprintln!(
                "tray: icon decode failed path={} error={error}",
                path.display()
            );
            (tray_icon(), "generated fallback".into())
        }
    }
}

#[cfg(target_os = "linux")]
fn platform_tray_icon() -> (tauri::image::Image<'static>, String) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("icons")
        .join("32x32.png");
    match tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png")) {
        Ok(icon) => (icon.to_owned(), path.display().to_string()),
        Err(error) => {
            eprintln!(
                "tray: icon decode failed path={} error={error}",
                path.display()
            );
            (tray_icon(), "generated fallback".into())
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_tray_icon() -> (tauri::image::Image<'static>, String) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("icons")
        .join("trayTemplate.png");
    match tauri::image::Image::from_bytes(include_bytes!("../icons/trayTemplate.png")) {
        Ok(icon) => (icon.to_owned(), path.display().to_string()),
        Err(error) => {
            eprintln!(
                "tray: template icon decode failed path={} error={error}",
                path.display()
            );
            (tray_icon(), "generated fallback".into())
        }
    }
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
                if let (Some(x), Some(y), Some(width), Some(height)) = (
                    number(position.and_then(|value| value.get("x"))),
                    number(position.and_then(|value| value.get("y"))),
                    number(size.and_then(|value| value.get("width"))),
                    number(size.and_then(|value| value.get("height"))),
                ) {
                    #[cfg(target_os = "macos")]
                    return Some((x as i32 + width as i32 - 380, y as i32 + height as i32 + 8));
                    #[cfg(target_os = "windows")]
                    return Some((
                        x as i32 + width as i32 - 380,
                        y as i32 - 560 - height as i32,
                    ));
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    return Some((x as i32 + width as i32 - 380, y as i32 + height as i32 + 8));
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
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    positioned_near_tray
}

fn show_main_window(app: &tauri::AppHandle) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let _ = window.unminimize();
    if let Err(error) = window.center() {
        eprintln!("window: center failed: {error}");
    }
    let shown = window.show().is_ok();
    let _ = window.set_focus();
    shown
}

#[cfg(debug_assertions)]
fn write_tray_icon_preview(icon: &tauri::image::Image<'_>) -> Result<(), String> {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("screenshots")
        .join("design-tray.png");
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
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    profiles::migrate_legacy_profiles();
    let cached = cache::load();
    let loaded_settings = settings::load();
    let start_hidden_in_tray = loaded_settings.start_hidden_in_tray;
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(AppState {
            snapshots: Arc::new(RwLock::new(cached.snapshots)),
            settings: Arc::new(RwLock::new(loaded_settings)),
            pace: Arc::new(std::sync::Mutex::new(pace::PaceTracker::default())),
            warnings: Arc::new(std::sync::Mutex::new(
                warnings::WarningTracker::from_persisted(cached.notified),
            )),
            tray: Arc::new(std::sync::Mutex::new(TrayRegistration::default())),
        })
        .invoke_handler(invoke_handler!())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        eprintln!("window: hide on close failed: {error}");
                    }
                }
            }
        })
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;
            if app.get_webview_window("main").is_none() {
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("")
                    .inner_size(380.0, 560.0)
                    .resizable(false)
                    .decorations(false)
                    .skip_taskbar(false)
                    .visible(false)
                    .build()?;
            }
            let history_path = app.path().app_data_dir()?.join("history.sqlite3");
            let history_state = history::HistoryState {
                store: Arc::new(
                    history::HistoryStore::new(history_path).map_err(std::io::Error::other)?,
                ),
                status: Arc::new(RwLock::new(history::HistoryStatus::default())),
            };
            app.manage(history_state.clone());
            history::start_backfill(history_state);
            commands::start_background_polling(
                app.app_handle().clone(),
                app.state::<AppState>().inner().clone(),
            );
            if !start_hidden_in_tray {
                show_main_window(&app.app_handle());
            }
            let menu = tauri::menu::MenuBuilder::new(app)
                .text("open", "Open Burnrate")
                .separator()
                .text("quit", "Quit")
                .build()?;
            let (icon, icon_path) = platform_tray_icon();
            let icon_width = icon.width();
            let icon_height = icon.height();
            #[cfg(debug_assertions)]
            let preview_icon = icon.clone();
            let tray_builder = tauri::tray::TrayIconBuilder::with_id("burnrate")
                .icon(icon)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => {
                        show_main_window(&app.app_handle());
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
                });
            #[cfg(target_os = "macos")]
            let tray_builder = tray_builder.icon_as_template(true);
            let tray_result = tray_builder.build(app);
            match tray_result {
                Ok(_) => {
                    let app_state = app.state::<AppState>();
                    if let Ok(mut registration) = app_state.tray.lock() {
                        registration.registered = true;
                        registration.icon_width = icon_width;
                        registration.icon_height = icon_height;
                    }
                    eprintln!(
                        "tray: registered=true icon={icon_path} size={icon_width}x{icon_height}"
                    );
                }
                Err(error) => {
                    eprintln!(
                        "tray: registered=false icon={icon_path} size={icon_width}x{icon_height} error={error}"
                    );
                    show_main_window(&app.app_handle());
                }
            }
            #[cfg(debug_assertions)]
            if std::env::var_os("BURNRATE_WRITE_TRAY_PREVIEW").is_some() {
                write_tray_icon_preview(&preview_icon).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Burnrate");
}
