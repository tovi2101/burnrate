//! Live provider adapters. Endpoint details intentionally mirror PROVIDERS.md; credentials are
//! read only into memory and are never included in errors or logs.
use crate::backoff::FailureBackoff;
use crate::models::*;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Error)]
pub enum LiveError {
    #[error("credentials unavailable")]
    Missing,
    #[error("request failed")]
    Request,
    #[error("response parse failed")]
    Parse,
}

static BACKOFF: OnceLock<Mutex<FailureBackoff>> = OnceLock::new();

fn can_try(provider: &str) -> bool {
    BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
        .map(|state| state.can_try(provider))
        .unwrap_or(true)
}

fn record_success(provider: &str) {
    if let Ok(mut state) = BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
    {
        state.record_success(provider);
    }
}

fn record_failure(provider: &str) {
    if let Ok(mut state) = BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
    {
        state.record_failure(provider);
    }
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

fn read_json(path: &Path) -> Result<Value, LiveError> {
    let data = std::fs::read(path).map_err(|_| LiveError::Missing)?;
    serde_json::from_slice(&data).map_err(|_| LiveError::Parse)
}

fn rfc3339(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value?
        .as_str()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|date| date.with_timezone(&Utc))
}

fn unix(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value?
        .as_i64()
        .and_then(|raw| Utc.timestamp_opt(raw, 0).single())
}

fn window(label: &str, pct: f64, reset: Option<DateTime<Utc>>) -> UsageWindow {
    UsageWindow {
        label: label.into(),
        used_pct: pct.clamp(0.0, 100.0),
        resets_at: reset,
    }
}

fn snapshot(
    provider: ProviderId,
    profile: &str,
    plan: Option<String>,
    windows: Vec<UsageWindow>,
) -> UsageSnapshot {
    UsageSnapshot {
        provider,
        profile_name: profile.into(),
        plan_name: plan,
        windows,
        fetched_at: Utc::now(),
        status: SnapshotStatus::Fresh,
        error_message: None,
    }
}

async fn claude(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let path = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".claude"))
        .join(".credentials.json");
    let body = read_json(&path)?;
    let oauth = body.get("claudeAiOauth").ok_or(LiveError::Missing)?;
    let mut token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .ok_or(LiveError::Missing)?
        .to_owned();
    let expired = oauth
        .get("expiresAt")
        .and_then(Value::as_i64)
        .map(|value| value <= Utc::now().timestamp_millis() + 30_000)
        .unwrap_or(false);
    if expired {
        if let Some(refresh) = oauth.get("refreshToken").and_then(Value::as_str) {
            let response = client
                .post("https://platform.claude.com/v1/oauth/token")
                .header("Accept", "application/json")
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh),
                    ("client_id", "9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
                ])
                .send()
                .await
                .map_err(|_| LiveError::Request)?;
            if response.status() == StatusCode::OK {
                token = response
                    .json::<Value>()
                    .await
                    .map_err(|_| LiveError::Parse)?
                    .get("access_token")
                    .and_then(Value::as_str)
                    .ok_or(LiveError::Parse)?
                    .into();
            }
        }
    }
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/2.1.238")
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if !response.status().is_success() {
        return Err(LiveError::Request);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    let mut windows = Vec::new();
    if let Some(five) = data.get("five_hour") {
        if let Some(pct) = five.get("utilization").and_then(Value::as_f64) {
            windows.push(window("5h", pct, rfc3339(five.get("resets_at"))));
        }
    }
    if let Some(weekly) = data.get("seven_day") {
        if let Some(pct) = weekly.get("utilization").and_then(Value::as_f64) {
            windows.push(window("Weekly", pct, rfc3339(weekly.get("resets_at"))));
        }
    }
    Ok(snapshot(
        ProviderId::Claude,
        profile,
        oauth
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        windows,
    ))
}

async fn codex(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"));
    let body = read_json(&root.join("auth.json"))?;
    let tokens = body.get("tokens").ok_or(LiveError::Missing)?;
    let mut token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or(LiveError::Missing)?
        .to_owned();
    let refresh = tokens.get("refresh_token").and_then(Value::as_str);
    let account = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut response = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .header("User-Agent", "CodexBar")
        .header("ChatGPT-Account-Id", account)
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        if let Some(refresh_token) = refresh {
            let refresh_response = client
                .post("https://auth.openai.com/oauth/token")
                .header("Content-Type", "application/json")
                .json(&json!({ "client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token", "refresh_token": refresh_token, "scope": "openid profile email" }))
                .send().await.map_err(|_| LiveError::Request)?;
            if refresh_response.status().is_success() {
                token = refresh_response
                    .json::<Value>()
                    .await
                    .map_err(|_| LiveError::Parse)?
                    .get("access_token")
                    .and_then(Value::as_str)
                    .ok_or(LiveError::Parse)?
                    .to_owned();
                response = client
                    .get("https://chatgpt.com/backend-api/wham/usage")
                    .bearer_auth(&token)
                    .header("Accept", "application/json")
                    .header("User-Agent", "CodexBar")
                    .header("ChatGPT-Account-Id", account)
                    .send()
                    .await
                    .map_err(|_| LiveError::Request)?;
            }
        }
    }
    if !response.status().is_success() {
        return Err(LiveError::Request);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    let mut windows = Vec::new();
    for (key, label) in [("primary_window", "5h"), ("secondary_window", "Weekly")] {
        if let Some(item) = data.pointer(&format!("/rate_limit/{key}")) {
            if let Some(pct) = item.get("used_percent").and_then(Value::as_f64) {
                windows.push(window(label, pct, unix(item.get("reset_at"))));
            }
        }
    }
    Ok(snapshot(
        ProviderId::Codex,
        profile,
        data.get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        windows,
    ))
}

async fn grok(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    if let Ok(data) = grok_rpc().await {
        let monthly = data
            .pointer("/monthlyLimit/val")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let used = data
            .pointer("/usage/totalUsed/val")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let pct = if monthly > 0.0 {
            used / monthly * 100.0
        } else {
            0.0
        };
        let reset = rfc3339(data.pointer("/billingCycle/billingPeriodEnd"));
        return Ok(snapshot(
            ProviderId::Grok,
            profile,
            Some("SuperGrok".into()),
            vec![window("Weekly", pct, reset)],
        ));
    }
    let root = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".grok"));
    let body = read_json(&root.join("auth.json"))?;
    let mut token = None;
    for entry in body
        .as_object()
        .into_iter()
        .flat_map(|object| object.values())
    {
        if entry.get("key").and_then(Value::as_str).is_some()
            && entry
                .get("auth_mode")
                .and_then(Value::as_str)
                .map(|mode| mode.eq_ignore_ascii_case("oidc"))
                .unwrap_or(false)
        {
            token = entry.get("key").and_then(Value::as_str);
            break;
        }
        if token.is_none() {
            token = entry.get("key").and_then(Value::as_str);
        }
    }
    let token = token.ok_or(LiveError::Missing)?;
    let response = client
        .get("https://cli-chat-proxy.grok.com/v1/billing?format=credits")
        .bearer_auth(token)
        .header("x-xai-token-auth", "xai-grok-cli")
        .header("Accept", "application/json")
        .header("User-Agent", "CodexBar")
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if !response.status().is_success() {
        return Err(LiveError::Request);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    let config = data.get("config").ok_or(LiveError::Parse)?;
    let pct = config
        .get("creditUsagePercent")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| {
            let used = config
                .pointer("/onDemandUsed/val")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let cap = config
                .pointer("/onDemandCap/val")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if cap > 0.0 {
                used / cap * 100.0
            } else {
                0.0
            }
        });
    let reset = rfc3339(config.pointer("/currentPeriod/end"))
        .or_else(|| rfc3339(config.get("billingPeriodEnd")));
    Ok(snapshot(
        ProviderId::Grok,
        profile,
        Some("SuperGrok".into()),
        vec![window("Weekly", pct, reset)],
    ))
}

async fn grok_rpc() -> Result<Value, LiveError> {
    let mut child = Command::new("grok")
        .args(["agent", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| LiveError::Missing)?;
    let mut stdin = child.stdin.take().ok_or(LiveError::Request)?;
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1","clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}});
    let billing = json!({"jsonrpc":"2.0","id":2,"method":"x.ai/billing","params":{}});
    stdin
        .write_all(format!("{}\n{}\n", initialize, billing).as_bytes())
        .await
        .map_err(|_| LiveError::Request)?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or(LiveError::Request)?;
    let mut lines = BufReader::new(stdout).lines();
    let result = timeout(Duration::from_secs(12), async {
        while let Some(line) = lines.next_line().await.map_err(|_| LiveError::Request)? {
            let message: Value = serde_json::from_str(&line).map_err(|_| LiveError::Parse)?;
            if message.get("id").and_then(Value::as_i64) == Some(2) {
                if message.get("error").is_some() {
                    return Err(LiveError::Request);
                }
                return message.get("result").cloned().ok_or(LiveError::Parse);
            }
        }
        Err(LiveError::Request)
    })
    .await
    .map_err(|_| LiveError::Request)??;
    let _ = child.kill().await;
    Ok(result)
}

async fn opencode(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let api_key = std::env::var("OPENCODE_API_KEY").ok();
    let manual_cookie = std::env::var("BURNRATE_OPENCODE_COOKIE").ok();
    if api_key.is_none() && manual_cookie.is_none() {
        return Err(LiveError::Missing);
    }
    let mut request = client
        .get("https://opencode.ai/zen/go/v1/usage")
        .header("Accept", "application/json")
        .header("User-Agent", "Burnrate");
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    if let Some(cookie) = manual_cookie {
        request = request.header("Cookie", cookie);
    }
    let response = request.send().await.map_err(|_| LiveError::Request)?;
    if !response.status().is_success() {
        return Err(LiveError::Request);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    let rolling = data.get("rollingUsage").ok_or(LiveError::Parse)?;
    let mut windows = vec![window(
        "5h",
        rolling
            .get("usagePercent")
            .and_then(Value::as_f64)
            .ok_or(LiveError::Parse)?,
        Some(
            Utc::now()
                + chrono::Duration::seconds(
                    rolling
                        .get("resetInSec")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                ),
        ),
    )];
    if let Some(weekly) = data.get("weeklyUsage") {
        if let Some(pct) = weekly.get("usagePercent").and_then(Value::as_f64) {
            windows.push(window(
                "Weekly",
                pct,
                Some(
                    Utc::now()
                        + chrono::Duration::seconds(
                            weekly
                                .get("resetInSec")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                        ),
                ),
            ));
        }
    }
    Ok(snapshot(
        ProviderId::Opencode,
        profile,
        Some("OpenCode Go".into()),
        windows,
    ))
}

async fn cursor(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let cookie = std::env::var("BURNRATE_CURSOR_COOKIE").map_err(|_| LiveError::Missing)?;
    let response = client
        .get("https://cursor.com/api/usage-summary")
        .header("Accept", "application/json")
        .header("Cookie", cookie)
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if !response.status().is_success() {
        return Err(LiveError::Request);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    let plan = data
        .pointer("/individualUsage/plan")
        .ok_or(LiveError::Parse)?;
    let pct = plan
        .get("totalPercentUsed")
        .and_then(Value::as_f64)
        .or_else(|| {
            let used = plan.get("used").and_then(Value::as_f64)?;
            let limit = plan.get("limit").and_then(Value::as_f64)?;
            (limit > 0.0).then_some(used / limit * 100.0)
        })
        .ok_or(LiveError::Parse)?;
    let reset = rfc3339(data.get("billingCycleEnd"));
    Ok(snapshot(
        ProviderId::Cursor,
        profile,
        data.get("membershipType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        vec![window("Monthly", pct, reset)],
    ))
}

pub async fn fetch_live() -> Vec<UsageSnapshot> {
    let client = match Client::builder().user_agent("Burnrate/0.1").build() {
        Ok(client) => client,
        Err(_) => return Vec::new(),
    };
    let mut snapshots = Vec::new();
    if can_try("claude") {
        match claude("Personal", &client).await {
            Ok(value) => {
                record_success("claude");
                snapshots.push(value);
            }
            Err(_) => record_failure("claude"),
        }
    }
    if can_try("codex") {
        match codex("Personal", &client).await {
            Ok(value) => {
                record_success("codex");
                snapshots.push(value);
            }
            Err(_) => record_failure("codex"),
        }
    }
    if can_try("grok") {
        match grok("Personal", &client).await {
            Ok(value) => {
                record_success("grok");
                snapshots.push(value);
            }
            Err(_) => record_failure("grok"),
        }
    }
    if can_try("opencode") {
        match opencode("Personal", &client).await {
            Ok(value) => {
                record_success("opencode");
                snapshots.push(value);
            }
            Err(_) => record_failure("opencode"),
        }
    }
    if can_try("cursor") {
        match cursor("Personal", &client).await {
            Ok(value) => {
                record_success("cursor");
                snapshots.push(value);
            }
            Err(_) => record_failure("cursor"),
        }
    }
    snapshots
}

pub fn detected() -> Vec<DetectedProvider> {
    let home = home_dir();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
        .join(".credentials.json");
    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
        .join("auth.json");
    let grok = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".grok"))
        .join("auth.json");
    vec![
        DetectedProvider {
            provider: ProviderId::Claude,
            state: if claude.exists() {
                Detection::Detected
            } else {
                Detection::NotLoggedIn
            },
            profile_name: Some("Personal".into()),
        },
        DetectedProvider {
            provider: ProviderId::Codex,
            state: if codex.exists() {
                Detection::Detected
            } else {
                Detection::NotLoggedIn
            },
            profile_name: Some("Personal".into()),
        },
        DetectedProvider {
            provider: ProviderId::Grok,
            state: if grok.exists() {
                Detection::Detected
            } else {
                Detection::NotLoggedIn
            },
            profile_name: Some("Personal".into()),
        },
        DetectedProvider {
            provider: ProviderId::Cursor,
            state: Detection::NotLoggedIn,
            profile_name: None,
        },
        DetectedProvider {
            provider: ProviderId::Opencode,
            state: Detection::NotLoggedIn,
            profile_name: None,
        },
    ]
}

pub fn rpc_probe_request(method: &str, id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":{}})
}
