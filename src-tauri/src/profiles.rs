use crate::models::ProviderId;
use keyring::Entry;
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;

const SERVICE: &str = "dev.burnrate.app";

fn provider_key(provider: &ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Grok => "grok",
        ProviderId::Cursor => "cursor",
        ProviderId::Opencode => "opencode",
    }
}

fn profile_account(provider: &ProviderId, name: &str) -> String {
    format!("profile:{}:{}", provider_key(provider), name)
}

fn index_account(provider: &ProviderId) -> String {
    format!("profiles:{}", provider_key(provider))
}

fn manual_account(provider: &ProviderId) -> String {
    format!("manual:{}", provider_key(provider))
}

fn home_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn current_credential(provider: &ProviderId) -> Option<String> {
    match provider {
        ProviderId::Claude => {
            let path = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".claude"))
                .join(".credentials.json");
            std::fs::read_to_string(path).ok()
        }
        ProviderId::Codex => {
            let path = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".codex"))
                .join("auth.json");
            std::fs::read_to_string(path).ok()
        }
        ProviderId::Grok => {
            let path = std::env::var_os("GROK_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".grok"))
                .join("auth.json");
            std::fs::read_to_string(path).ok()
        }
        ProviderId::Cursor => std::env::var("BURNRATE_CURSOR_COOKIE")
            .ok()
            .or_else(|| manual_value(provider))
            .map(|cookie| serde_json::json!({ "cookie": cookie }).to_string()),
        ProviderId::Opencode => std::env::var("OPENCODE_API_KEY")
            .ok()
            .or_else(|| {
                let path = home_dir()
                    .join(".local")
                    .join("share")
                    .join("opencode")
                    .join("opencode.db");
                Connection::open(path)
                    .ok()?
                    .query_row(
                        "SELECT access_token FROM account ORDER BY time_updated DESC LIMIT 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
            })
            .map(|api_key| serde_json::json!({ "api_key": api_key }).to_string())
            .or_else(|| {
                manual_value(provider)
                    .map(|cookie| serde_json::json!({ "cookie": cookie }).to_string())
            }),
    }
}

pub fn manual_value(provider: &ProviderId) -> Option<String> {
    Entry::new(SERVICE, &manual_account(provider))
        .ok()?
        .get_password()
        .ok()
}

pub fn save_manual(provider: &ProviderId, value: &str) -> Result<(), String> {
    if !matches!(provider, ProviderId::Cursor | ProviderId::Opencode) {
        return Err("Manual web fallback is only available for Cursor and OpenCode".into());
    }
    let value = value.trim();
    if value.is_empty() || value.len() > 16_384 {
        return Err("Manual credential is invalid".into());
    }
    Entry::new(SERVICE, &manual_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(value)
        .map_err(|_| "OS keyring write failed".to_string())
}

pub fn delete_manual(provider: &ProviderId) -> Result<(), String> {
    Entry::new(SERVICE, &manual_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .delete_credential()
        .map_err(|_| "OS keyring delete failed".to_string())
}

pub fn credential(provider: &ProviderId, name: &str) -> Option<String> {
    if name == "Personal" {
        return current_credential(provider);
    }
    Entry::new(SERVICE, &profile_account(provider, name))
        .ok()?
        .get_password()
        .ok()
}

pub fn list(provider: &ProviderId) -> Vec<String> {
    let mut names = vec!["Personal".to_string()];
    let Ok(entry) = Entry::new(SERVICE, &index_account(provider)) else {
        return names;
    };
    let Ok(raw) = entry.get_password() else {
        return names;
    };
    let Ok(index) = serde_json::from_str::<Vec<String>>(&raw) else {
        return names;
    };
    for name in index {
        if name != "Personal" && !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

pub fn save(provider: &ProviderId, name: &str) -> Result<(), String> {
    let raw = current_credential(provider)
        .ok_or_else(|| "No CLI credential or manual fallback found".to_string())?;
    // Validate before touching the keyring so malformed profile material never gets stored.
    let _: Value = serde_json::from_str(&raw)
        .map_err(|_| "Credential material could not be parsed".to_string())?;
    Entry::new(SERVICE, &profile_account(provider, name))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&raw)
        .map_err(|_| "OS keyring write failed".to_string())?;
    let mut names = list(provider);
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_string());
    }
    let encoded =
        serde_json::to_string(&names[1..]).map_err(|_| "Profile index failed".to_string())?;
    Entry::new(SERVICE, &index_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
        .map_err(|_| "OS keyring write failed".to_string())
}

pub fn delete(provider: &ProviderId, name: &str) -> Result<(), String> {
    if name == "Personal" {
        return Err("The current login cannot be deleted".into());
    }
    Entry::new(SERVICE, &profile_account(provider, name))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .delete_credential()
        .map_err(|_| "OS keyring delete failed".to_string())?;
    let names: Vec<String> = list(provider)
        .into_iter()
        .filter(|existing| existing != "Personal" && existing != name)
        .collect();
    let encoded = serde_json::to_string(&names).map_err(|_| "Profile index failed".to_string())?;
    Entry::new(SERVICE, &index_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
        .map_err(|_| "OS keyring write failed".to_string())
}
