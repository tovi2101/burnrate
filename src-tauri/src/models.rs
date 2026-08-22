use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub enabled: BTreeMap<String, bool>,
    #[serde(alias = "refresh_seconds")]
    pub refresh_seconds: u64,
    #[serde(alias = "launch_at_login")]
    pub launch_at_login: bool,
    #[serde(default, alias = "start_hidden_in_tray")]
    pub start_hidden_in_tray: bool,
    pub theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            enabled: BTreeMap::from([
                ("claude".into(), true),
                ("codex".into(), true),
                ("grok".into(), true),
                ("cursor".into(), true),
                ("opencode".into(), true),
            ]),
            refresh_seconds: 60,
            launch_at_login: false,
            start_hidden_in_tray: false,
            theme: "dark".into(),
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Cursor => "cursor",
            Self::Opencode => "opencode",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Claude,
    Codex,
    Grok,
    Cursor,
    Opencode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageWindow {
    pub label: String,
    pub used_pct: f64,
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pace_limit_minutes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider: ProviderId,
    pub profile_name: String,
    pub plan_name: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub fetched_at: DateTime<Utc>,
    pub status: SnapshotStatus,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotStatus {
    Fresh,
    Stale,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Detection {
    Detected,
    NotInstalled,
    NotLoggedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedProvider {
    pub provider: ProviderId,
    pub state: Detection,
    pub profile_name: Option<String>,
}
