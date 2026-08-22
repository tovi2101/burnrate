use crate::models::ProviderId;
use keyring::Entry;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE: &str = "dev.burnrate.app";
const PROFILE_REFERENCE_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingAccount {
    profile_name: String,
    original_identity: String,
    isolated_source: CredentialSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSetup {
    pub supported: bool,
    pub pending: bool,
    pub identity: Option<String>,
    pub suggested_name: String,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountResult {
    pub profile_name: String,
    pub profiles: Vec<String>,
    pub already_saved: bool,
    pub message: String,
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

fn pending_account(provider: &ProviderId) -> String {
    format!("pending:{}", provider_key(provider))
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

fn supports_isolated_accounts(provider: &ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex | ProviderId::Grok)
}

fn unsupported_explanation(provider: &ProviderId) -> Option<String> {
    match provider {
        ProviderId::Cursor => Some(
            "Cursor does not expose isolated CLI config directories, so Burnrate cannot safely keep multiple rotating logins active."
                .into(),
        ),
        ProviderId::Opencode => Some(
            "OpenCode does not expose an isolated credential source that Burnrate can delegate refresh to safely, so multiple live accounts are unavailable."
                .into(),
        ),
        ProviderId::Claude | ProviderId::Codex | ProviderId::Grok => None,
    }
}

fn managed_accounts_dir() -> PathBuf {
    let config = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
    };
    config.join("Burnrate").join("accounts")
}

fn isolated_source(
    provider: &ProviderId,
    source: &CredentialSource,
    identity: &str,
) -> Result<CredentialSource, String> {
    let CredentialSource::CliFile { path } = source else {
        return Err("This provider cannot isolate its current credential source".into());
    };
    let contents = std::fs::read(path)
        .map_err(|_| "The current CLI credential file could not be read".to_string())?;
    let permissions = std::fs::metadata(path)
        .map_err(|_| "The current CLI credential permissions could not be read".to_string())?
        .permissions();
    let mut hasher = DefaultHasher::new();
    provider_key(provider).hash(&mut hasher);
    identity.to_ascii_lowercase().hash(&mut hasher);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = managed_accounts_dir()
        .join(provider_key(provider))
        .join(format!("{:016x}-{stamp:x}", hasher.finish()));
    std::fs::create_dir_all(&directory)
        .map_err(|_| "The isolated CLI config directory could not be created".to_string())?;
    let filename = path
        .file_name()
        .ok_or_else(|| "The current credential path is invalid".to_string())?;
    let destination = directory.join(filename);
    let temporary = directory.join(format!(".{}.tmp", filename.to_string_lossy()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| "The isolated credential file could not be created".to_string())?;
    file.write_all(&contents)
        .and_then(|_| file.sync_all())
        .map_err(|_| "The isolated credential file could not be written".to_string())?;
    std::fs::set_permissions(&temporary, permissions)
        .map_err(|_| "The isolated credential permissions could not be preserved".to_string())?;
    std::fs::rename(&temporary, &destination)
        .map_err(|_| "The isolated credential file could not be committed atomically".to_string())?;
    Ok(CredentialSource::CliFile { path: destination })
}

fn remove_isolated_source(source: &CredentialSource) {
    let CredentialSource::CliFile { path } = source else {
        return;
    };
    let managed = normalized_source_path(&managed_accounts_dir());
    let candidate = normalized_source_path(path);
    if !candidate.starts_with(&managed) {
        return;
    }
    if std::fs::remove_file(path).is_ok() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
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

fn stored_names(provider: &ProviderId) -> Vec<String> {
    let Ok(entry) = Entry::new(SERVICE, &index_account(provider)) else {
        return Vec::new();
    };
    let Ok(raw) = entry.get_password() else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_str::<Vec<String>>(&raw) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for name in index {
        if !name.is_empty()
            && !name.eq_ignore_ascii_case("personal")
            && !name.eq_ignore_ascii_case("all")
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

fn write_index(provider: &ProviderId, names: &[String]) -> Result<(), String> {
    let encoded = serde_json::to_string(names).map_err(|_| "Profile index failed".to_string())?;
    Entry::new(SERVICE, &index_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
        .map_err(|_| "OS keyring write failed".to_string())
}

fn save_reference(
    provider: &ProviderId,
    name: &str,
    reference: &StoredProfileReference,
) -> Result<(), String> {
    let encoded = serde_json::to_string(reference)
        .map_err(|_| "Credential source reference could not be encoded".to_string())?;
    Entry::new(SERVICE, &profile_account(provider, name))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
        .map_err(|_| "OS keyring write failed".to_string())?;
    let mut names = stored_names(provider);
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_owned());
    }
    write_index(provider, &names)
}

fn pending_reference(provider: &ProviderId) -> Option<PendingAccount> {
    let raw = Entry::new(SERVICE, &pending_account(provider))
        .ok()?
        .get_password()
        .ok()?;
    serde_json::from_str(&raw).ok()
}

fn clear_pending_reference(provider: &ProviderId) {
    if let Ok(entry) = Entry::new(SERVICE, &pending_account(provider)) {
        let _ = entry.delete_credential();
    }
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
    if let Some(identity) = account_identity(provider, name) {
        return Some(format!(
            "{}:account:{}",
            provider_key(provider),
            identity.trim().to_ascii_lowercase()
        ));
    }
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

fn claude_cli_identity(source: &CredentialSource) -> Option<String> {
    let mut command = Command::new("claude");
    command.args(["auth", "status"]);
    if let CredentialSource::CliFile { path } = source {
        if let Some(root) = path.parent() {
            command.env("CLAUDE_CONFIG_DIR", root);
        }
    }
    let output = command.output().ok()?;
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
        ProviderId::Claude => body
            .pointer("/claudeAiOauth/accountUuid")
            .or_else(|| body.pointer("/claudeAiOauth/organizationUuid"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| claude_cli_identity(source)),
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
    build_reference_from_source(provider, source)
}

fn build_reference_from_source(
    provider: &ProviderId,
    source: CredentialSource,
) -> Option<StoredProfileReference> {
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
        for name in stored_names(&provider) {
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
    let mut names = stored_names(provider);
    let current_identity = build_reference(provider).map(|reference| reference.account_identity);
    let current_is_named = current_identity.as_deref().is_some_and(|identity| {
        names.iter().any(|name| {
            stored_reference(provider, name)
                .is_some_and(|reference| reference.account_identity.eq_ignore_ascii_case(identity))
        })
    });
    if !current_is_named {
        names.insert(0, "Personal".to_string());
    }
    names
}

pub fn save(provider: &ProviderId, name: &str) -> Result<(), String> {
    let reference = build_reference(provider)
        .ok_or_else(|| "No CLI credential or manual fallback found".to_string())?;
    if stored_names(provider).iter().any(|existing| {
        existing != name
            && stored_reference(provider, existing).is_some_and(|stored| {
                stored
                    .account_identity
                    .eq_ignore_ascii_case(&reference.account_identity)
            })
    }) {
        return Err("This account is already saved".into());
    }
    save_reference(provider, name, &reference)
}

pub fn delete(provider: &ProviderId, name: &str) -> Result<(), String> {
    if name == "Personal" {
        return Err("The current login cannot be deleted".into());
    }
    let deleted_reference = stored_reference(provider, name);
    Entry::new(SERVICE, &profile_account(provider, name))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .delete_credential()
        .map_err(|_| "OS keyring delete failed".to_string())?;
    let names: Vec<String> = stored_names(provider)
        .into_iter()
        .filter(|existing| existing != name)
        .collect();
    write_index(provider, &names)?;
    if let Some(reference) = deleted_reference {
        let source_still_used = names.iter().any(|existing| {
            stored_reference(provider, existing)
                .is_some_and(|stored| stored.source == reference.source)
        });
        if !source_still_used {
            remove_isolated_source(&reference.source);
        }
    }
    Ok(())
}

pub fn is_add_pending(provider: &ProviderId) -> bool {
    pending_reference(provider).is_some()
}

pub fn account_setup(provider: &ProviderId) -> AccountSetup {
    if let Some(pending) = pending_reference(provider) {
        return AccountSetup {
            supported: true,
            pending: true,
            identity: Some(pending.original_identity),
            suggested_name: pending.profile_name,
            explanation: None,
        };
    }
    let reference = build_reference(provider);
    let identity = reference.map(|value| value.account_identity);
    AccountSetup {
        supported: supports_isolated_accounts(provider),
        pending: false,
        suggested_name: identity
            .as_deref()
            .map(suggested_profile_name)
            .unwrap_or_else(|| "Account".into()),
        identity,
        explanation: unsupported_explanation(provider),
    }
}

fn suggested_profile_name(identity: &str) -> String {
    let trimmed = identity.trim();
    if trimmed.contains('@') && trimmed.len() <= 48 {
        return trimmed.to_owned();
    }
    let safe = trimmed
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-' || *character == '_')
        .take(16)
        .collect::<String>();
    if safe.is_empty() {
        "Account".into()
    } else {
        format!("Account {safe}")
    }
}

fn unique_profile_name(provider: &ProviderId, preferred: &str) -> String {
    let names = stored_names(provider);
    if !names.iter().any(|name| name.eq_ignore_ascii_case(preferred)) {
        return preferred.to_owned();
    }
    for suffix in 2.. {
        let candidate = format!("{preferred} {suffix}");
        if !names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    unreachable!()
}

pub fn begin_add_account(provider: &ProviderId, profile_name: &str) -> Result<AccountSetup, String> {
    if !supports_isolated_accounts(provider) {
        return Err(unsupported_explanation(provider)
            .unwrap_or_else(|| "Multiple accounts are unavailable for this provider".into()));
    }
    let profile_name = profile_name.trim();
    if profile_name.is_empty()
        || profile_name.len() > 48
        || profile_name.eq_ignore_ascii_case("personal")
        || profile_name.eq_ignore_ascii_case("all")
    {
        return Err("Profile name is invalid".into());
    }
    if pending_reference(provider).is_some() {
        return Ok(account_setup(provider));
    }
    let current = build_reference(provider)
        .ok_or_else(|| "No current CLI login was found".to_string())?;
    let existing_name = stored_names(provider).into_iter().find(|name| {
        stored_reference(provider, name).is_some_and(|stored| {
            stored
                .account_identity
                .eq_ignore_ascii_case(&current.account_identity)
        })
    });
    let profile_name = existing_name.unwrap_or_else(|| profile_name.to_owned());
    let isolated = isolated_source(provider, &current.source, &current.account_identity)?;
    let pending = PendingAccount {
        profile_name: profile_name.clone(),
        original_identity: current.account_identity.clone(),
        isolated_source: isolated,
    };
    let encoded = serde_json::to_string(&pending)
        .map_err(|_| "Pending account could not be encoded".to_string())?;
    if let Err(error) = Entry::new(SERVICE, &pending_account(provider))
        .map_err(|_| "OS keyring unavailable".to_string())?
        .set_password(&encoded)
        .map_err(|_| "OS keyring write failed".to_string())
    {
        remove_isolated_source(&pending.isolated_source);
        return Err(error);
    }
    eprintln!(
        "profiles: current {} login preserved in an isolated CLI credential source; polling paused until detection completes",
        provider_key(provider)
    );
    Ok(AccountSetup {
        supported: true,
        pending: true,
        identity: Some(current.account_identity),
        suggested_name: profile_name,
        explanation: None,
    })
}

pub fn cancel_add_account(provider: &ProviderId) -> Result<(), String> {
    let pending = pending_reference(provider)
        .ok_or_else(|| "No add-account flow is pending".to_string())?;
    let current = build_reference(provider)
        .ok_or_else(|| "The current CLI login could not be read".to_string())?;
    if !current
        .account_identity
        .eq_ignore_ascii_case(&pending.original_identity)
    {
        return Err(
            "Detect the new login first so the previous account is not lost".into(),
        );
    }
    remove_isolated_source(&pending.isolated_source);
    clear_pending_reference(provider);
    eprintln!(
        "profiles: cancelled pending {} account flow; isolated credential copy deleted",
        provider_key(provider)
    );
    Ok(())
}

pub fn detect_new_account(provider: &ProviderId) -> Result<AddAccountResult, String> {
    let pending = pending_reference(provider)
        .ok_or_else(|| "Start the add-account flow first".to_string())?;
    let current = build_reference(provider)
        .ok_or_else(|| "No current CLI login was detected yet".to_string())?;
    if current
        .account_identity
        .eq_ignore_ascii_case(&pending.original_identity)
    {
        return Err("This account is already saved".into());
    }

    let mut original_names = stored_names(provider)
        .into_iter()
        .filter(|name| {
            stored_reference(provider, name).is_some_and(|stored| {
                stored
                    .account_identity
                    .eq_ignore_ascii_case(&pending.original_identity)
            })
        })
        .collect::<Vec<_>>();
    if original_names.is_empty() {
        original_names.push(pending.profile_name.clone());
    }
    let original = StoredProfileReference {
        version: PROFILE_REFERENCE_VERSION,
        source: pending.isolated_source.clone(),
        account_identity: pending.original_identity.clone(),
    };
    for name in original_names {
        save_reference(provider, &name, &original)?;
    }

    let existing_new = stored_names(provider).into_iter().find(|name| {
        stored_reference(provider, name).is_some_and(|stored| {
            stored
                .account_identity
                .eq_ignore_ascii_case(&current.account_identity)
        })
    });
    let (profile_name, already_saved) = if let Some(name) = existing_new {
        (name, true)
    } else {
        let preferred = suggested_profile_name(&current.account_identity);
        let name = unique_profile_name(provider, &preferred);
        save_reference(provider, &name, &current)?;
        (name, false)
    };
    clear_pending_reference(provider);
    eprintln!(
        "profiles: detected a distinct {} account; previous login remains delegated to its isolated CLI source",
        provider_key(provider)
    );
    Ok(AddAccountResult {
        profile_name,
        profiles: list(provider),
        already_saved,
        message: if already_saved {
            "This account is already saved".into()
        } else {
            "New account detected and saved".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::suggested_profile_name;

    #[test]
    fn email_identity_is_used_as_the_profile_name() {
        assert_eq!(suggested_profile_name("user@example.com"), "user@example.com");
    }

    #[test]
    fn opaque_identity_gets_a_bounded_account_label() {
        assert_eq!(
            suggested_profile_name("550e8400-e29b-41d4-a716-446655440000"),
            "Account 550e8400-e29b-41"
        );
    }
}
