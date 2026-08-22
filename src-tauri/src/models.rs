use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub enabled: BTreeMap<String, bool>,
    pub refresh_seconds: u64,
    pub launch_at_login: bool,
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
            theme: "dark".into(),
        }
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
