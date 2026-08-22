use crate::models::AppSettings;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    let root = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".config"))
                    .unwrap_or_else(|| PathBuf::from("."))
            })
    };
    root.join("burnrate").join("settings.json")
}

fn summary(settings: &AppSettings) -> String {
    let enabled = settings
        .enabled
        .iter()
        .map(|(provider, value)| format!("{provider}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "enabled=[{enabled}] refresh_seconds={} launch_at_login={} start_hidden_in_tray={} limit_warnings={} warning_thresholds={:?} theme={}",
        settings.refresh_seconds,
        settings.launch_at_login,
        settings.start_hidden_in_tray,
        settings.limit_warnings,
        settings.warning_thresholds,
        settings.theme
    )
}

pub fn load() -> AppSettings {
    let file = path();
    let settings = std::fs::read(&file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AppSettings>(&bytes).ok())
        .unwrap_or_default();
    eprintln!(
        "settings: startup read path={} found={} {}",
        file.display(),
        file.exists(),
        summary(&settings)
    );
    settings
}

pub fn save(settings: &AppSettings) -> Result<(), String> {
    let file = path();
    let parent = file
        .parent()
        .ok_or_else(|| "settings path has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    let temporary = file.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &file).map_err(|error| error.to_string())?;
    let modified = std::fs::metadata(&file)
        .and_then(|metadata| metadata.modified())
        .map(|time| format!("{time:?}"))
        .unwrap_or_else(|_| "unknown".into());
    eprintln!(
        "settings: write path={} modified={} {}",
        file.display(),
        modified,
        summary(settings)
    );
    Ok(())
}
