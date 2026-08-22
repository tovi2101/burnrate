use crate::models::ProviderId;
use keyring::Entry;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE: &str = "dev.burnrate.app";
const PROFILE_REFERENCE_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CredentialSource {
    CliFile { path: PathBuf },
    ManualKeyring,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProfileReference {
    version: u8,
    source: CredentialSource,
    account_identity: String,
}

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

fn current_credential_path(provider: &ProviderId) -> Option<PathBuf> {
    match provider {
        ProviderId::Claude => Some(
            std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".claude"))
                .join(".credentials.json"),
        ),
        ProviderId::Codex => Some(
            std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".codex"))
                .join("auth.json"),
        ),
        ProviderId::Grok => Some(
            std::env::var_os("GROK_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".grok"))
                .join("auth.json"),
        ),
        ProviderId::Cursor | ProviderId::Opencode => None,
    }
}

fn current_source(provider: &ProviderId) -> CredentialSource {
    current_credential_path(provider)
        .map(|path| CredentialSource::CliFile { path })
        .unwrap_or(CredentialSource::ManualKeyring)
}

fn read_source(provider: &ProviderId, source: &CredentialSource) -> Option<String> {
    match source {
        CredentialSource::CliFile { path } => std::fs::read_to_string(path).ok(),
        CredentialSource::ManualKeyring => current_manual_credential(provider),
    }
}

fn current_manual_credential(provider: &ProviderId) -> Option<String> {
    match provider {
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
        ProviderId::Claude | ProviderId::Codex | ProviderId::Grok => None,
    }
}

pub fn current_credential(provider: &ProviderId) -> Option<String> {
    read_source(provider, &current_source(provider))
}

fn stored_reference(provider: &ProviderId, name: &str) -> Option<StoredProfileReference> {
    let raw = Entry::new(SERVICE, &profile_account(provider, name))
        .ok()?
        .get_password()
        .ok()?;
    serde_json::from_str(&raw).ok()
}

fn source(provider: &ProviderId, name: &str) -> Option<CredentialSource> {
    if name == "Personal" {
        Some(current_source(provider))
    } else {
        Some(stored_reference(provider, name)?.source)
    }
}

fn normalized_source_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = resolved.to_string_lossy().into_owned();
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

pub fn source_key(provider: &ProviderId, name: &str) -> Option<String> {
    Some(match source(provider, name)? {
        CredentialSource::CliFile { path } => {
            format!(
                "{}:file:{}",
                provider_key(provider),
                normalized_source_path(&path)
            )
        }
        CredentialSource::ManualKeyring => format!("{}:manual", provider_key(provider)),
    })
}

pub fn source_root(provider: &ProviderId, name: &str) -> Option<PathBuf> {
    match source(provider, name)? {
        CredentialSource::CliFile { path } => path.parent().map(Path::to_path_buf),
        CredentialSource::ManualKeyring => None,
    }
}

pub fn account_identity(provider: &ProviderId, name: &str) -> Option<String> {
    if name == "Personal" {
        Some(build_reference(provider)?.account_identity)
    } else {
        Some(stored_reference(provider, name)?.account_identity)
    }
}

fn claude_cli_identity() -> Option<String> {
    let output = Command::new("claude")
        .args(["auth", "status"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body: Value = serde_json::from_slice(&output.stdout).ok()?;
    body.get("orgId")
        .or_else(|| body.get("email"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase())
}

fn account_identity_from_raw(
    provider: &ProviderId,
    raw: &str,
    source: &CredentialSource,
) -> String {
    let body: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let discovered = match provider {
        ProviderId::Claude => claude_cli_identity().or_else(|| {
            body.pointer("/claudeAiOauth/organizationUuid")
                .or_else(|| body.pointer("/claudeAiOauth/accountUuid"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
        ProviderId::Codex => body
            .pointer("/tokens/account_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ProviderId::Grok => body.as_object().and_then(|entries| {
            entries.values().find_map(|entry| {
                entry
                    .get("user_id")
                    .or_else(|| entry.get("team_id"))
                    .or_else(|| entry.get("email"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        }),
        ProviderId::Cursor | ProviderId::Opencode => None,
    };
    discovered.unwrap_or_else(|| match source {
        CredentialSource::CliFile { path } => normalized_source_path(path),
        CredentialSource::ManualKeyring => format!("{}:manual", provider_key(provider)),
    })
}

fn build_reference(provider: &ProviderId) -> Option<StoredProfileReference> {
    let source = current_source(provider);
    let raw = read_source(provider, &source)?;
    serde_json::from_str::<Value>(&raw).ok()?;
    Some(StoredProfileReference {
        version: PROFILE_REFERENCE_VERSION,
        account_identity: account_identity_from_raw(provider, &raw, &source),
        source,
    })
}

pub fn migrate_legacy_profiles() {
    for provider in [
        ProviderId::Claude,
        ProviderId::Codex,
        ProviderId::Grok,
        ProviderId::Cursor,
        ProviderId::Opencode,
    ] {
        let replacement = build_reference(&provider);
        let mut claude_deleted = 0_usize;
        for name in list(&provider)
            .into_iter()
            .filter(|name| name != "Personal")
        {
            let Ok(entry) = Entry::new(SERVICE, &profile_account(&provider, &name)) else {
                continue;
            };
            let Ok(raw) = entry.get_password() else {
                continue;
            };
            if serde_json::from_str::<StoredProfileReference>(&raw).is_ok() {
                continue;
            }
            if entry.delete_credential().is_err() {
                continue;
            }
            if provider == ProviderId::Claude {
                claude_deleted += 1;
            }
            if let Some(reference) = replacement.as_ref() {
                if let Ok(encoded) = serde_json::to_string(reference) {
                    let _ = entry.set_password(&encoded);
                }
            }
        }
        if claude_deleted > 0 {
            eprintln!(
                "profiles: deleted {claude_deleted} stale Claude token keyring entry and relinked profile labels to the CLI credential source"
            );
        }
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
    let reference = stored_reference(provider, name)?;
    read_source(provider, &reference.source)
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
    let reference = build_reference(provider)
        .ok_or_else(|| "No CLI credential or manual fallback found".to_string())?;
    let encoded = serde_json::to_string(&reference)
        .map_err(|_| "Credential source reference could not be encoded".to_string())?;
    Entry::new(SERVICE, &profile_account(provider, name))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
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
