//! Live provider adapters. Endpoint details intentionally mirror PROVIDERS.md; credentials are
//! read only into memory and are never included in errors or logs.
use crate::backoff::FailureBackoff;
use crate::models::*;
use crate::profiles;
use crate::providers::{Provider, ProviderError};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{header::RETRY_AFTER, Client, Response, StatusCode};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const DEFAULT_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LiveError {
    #[error("credentials unavailable")]
    Missing,
    #[error("request failed")]
    Request,
    #[error("rate limited for {0:?}")]
    RateLimited(Duration),
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

fn is_rate_limited(provider: &str) -> bool {
    BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
        .map(|state| state.is_rate_limited(provider))
        .unwrap_or(false)
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

fn record_rate_limit(provider: &str, retry_after: Duration) {
    if let Ok(mut state) = BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
    {
        state.record_rate_limit(provider, retry_after);
    }
}

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .parse::<u64>()
                .map(Duration::from_secs)
                .ok()
                .or_else(|| {
                    DateTime::parse_from_rfc2822(value).ok().and_then(|date| {
                        date.with_timezone(&Utc)
                            .signed_duration_since(Utc::now())
                            .to_std()
                            .ok()
                    })
                })
        })
        .unwrap_or(DEFAULT_RATE_LIMIT_BACKOFF)
}

fn response_error(response: &Response) -> Option<LiveError> {
    if response.status().is_success() {
        None
    } else if response.status() == StatusCode::TOO_MANY_REQUESTS {
        Some(LiveError::RateLimited(retry_after(response)))
    } else {
        Some(LiveError::Request)
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

fn profile_json(provider: &ProviderId, profile: &str) -> Result<Value, LiveError> {
    let raw = profiles::credential(provider, profile).ok_or(LiveError::Missing)?;
    serde_json::from_str(&raw).map_err(|_| LiveError::Parse)
}

fn persist_profile(provider: &ProviderId, profile: &str, body: &Value) {
    if profile == "Personal" {
        return;
    }
    if let Ok(raw) = serde_json::to_string(body) {
        if let Ok(entry) = keyring::Entry::new(
            "dev.burnrate.app",
            &format!("profile:{}:{}", provider_key(provider), profile),
        ) {
            let _ = entry.set_password(&raw);
        }
    }
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
    let mut body = profile_json(&ProviderId::Claude, profile)?;
    let oauth = body
        .get("claudeAiOauth")
        .cloned()
        .ok_or(LiveError::Missing)?;
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
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                return Err(LiveError::RateLimited(retry_after(&response)));
            }
            if response.status() == StatusCode::OK {
                let refreshed = response
                    .json::<Value>()
                    .await
                    .map_err(|_| LiveError::Parse)?;
                token = refreshed
                    .get("access_token")
                    .and_then(Value::as_str)
                    .ok_or(LiveError::Parse)?
                    .into();
                if let Some(oauth_body) = body.get_mut("claudeAiOauth") {
                    if let Some(object) = oauth_body.as_object_mut() {
                        object.insert("accessToken".into(), Value::String(token.clone()));
                        if let Some(expires_in) =
                            refreshed.get("expires_in").and_then(Value::as_i64)
                        {
                            object.insert(
                                "expiresAt".into(),
                                Value::from(Utc::now().timestamp_millis() + expires_in * 1000),
                            );
                        }
                    }
                }
                persist_profile(&ProviderId::Claude, profile, &body);
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
    if let Some(error) = response_error(&response) {
        return Err(error);
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
        body.get("claudeAiOauth")
            .unwrap_or(&oauth)
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        windows,
    ))
}

async fn codex(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let mut body = profile_json(&ProviderId::Codex, profile)?;
    let tokens = body.get("tokens").cloned().ok_or(LiveError::Missing)?;
    let mut token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or(LiveError::Missing)?
        .to_owned();
    let refresh = tokens
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut response = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .header("User-Agent", "CodexBar")
        .header("ChatGPT-Account-Id", &account)
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        if let Some(refresh_token) = refresh.as_deref() {
            let refresh_response = client
                .post("https://auth.openai.com/oauth/token")
                .header("Content-Type", "application/json")
                .json(&json!({ "client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token", "refresh_token": refresh_token, "scope": "openid profile email" }))
                .send().await.map_err(|_| LiveError::Request)?;
            if refresh_response.status() == StatusCode::TOO_MANY_REQUESTS {
                return Err(LiveError::RateLimited(retry_after(&refresh_response)));
            }
            if refresh_response.status().is_success() {
                let refreshed = refresh_response
                    .json::<Value>()
                    .await
                    .map_err(|_| LiveError::Parse)?;
                token = refreshed
                    .get("access_token")
                    .and_then(Value::as_str)
                    .ok_or(LiveError::Parse)?
                    .to_owned();
                if let Some(tokens_body) = body.get_mut("tokens") {
                    if let Some(object) = tokens_body.as_object_mut() {
                        object.insert("access_token".into(), Value::String(token.clone()));
                        if let Some(refresh_token) = refreshed.get("refresh_token") {
                            object.insert("refresh_token".into(), refresh_token.clone());
                        }
                    }
                }
                persist_profile(&ProviderId::Codex, profile, &body);
                response = client
                    .get("https://chatgpt.com/backend-api/wham/usage")
                    .bearer_auth(&token)
                    .header("Accept", "application/json")
                    .header("User-Agent", "CodexBar")
                    .header("ChatGPT-Account-Id", &account)
                    .send()
                    .await
                    .map_err(|_| LiveError::Request)?;
            }
        }
    }
    if let Some(error) = response_error(&response) {
        return Err(error);
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
    if profile == "Personal" {
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
    }
    let body = profile_json(&ProviderId::Grok, profile)?;
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
    if let Some(error) = response_error(&response) {
        return Err(error);
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
    let (api_key, manual_cookie) = if profile == "Personal" {
        (
            std::env::var("OPENCODE_API_KEY")
                .ok()
                .or_else(opencode_db_token),
            std::env::var("BURNRATE_OPENCODE_COOKIE")
                .ok()
                .or_else(|| profiles::manual_value(&ProviderId::Opencode)),
        )
    } else {
        let body = profile_json(&ProviderId::Opencode, profile)?;
        (
            body.get("api_key")
                .and_then(Value::as_str)
                .map(str::to_owned),
            body.get("cookie")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    };
    fetch_opencode_with_credentials(
        profile,
        client,
        api_key,
        manual_cookie,
        "https://opencode.ai/zen/go/v1/usage",
    )
    .await
}

pub async fn fetch_opencode_with_credentials(
    profile: &str,
    client: &Client,
    api_key: Option<String>,
    manual_cookie: Option<String>,
    endpoint: &str,
) -> Result<UsageSnapshot, LiveError> {
    if api_key.is_none() && manual_cookie.is_none() {
        return Err(LiveError::Missing);
    }
    let mut request = client
        .get(endpoint)
        .header("Accept", "application/json")
        .header("User-Agent", "Burnrate");
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    if let Some(cookie) = manual_cookie {
        request = request.header("Cookie", cookie);
    }
    let response = request.send().await.map_err(|_| LiveError::Request)?;
    if let Some(error) = response_error(&response) {
        return Err(error);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    parse_opencode_usage_response(profile, &data)
}

pub fn parse_opencode_usage_response(
    profile: &str,
    data: &Value,
) -> Result<UsageSnapshot, LiveError> {
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

fn opencode_db_token() -> Option<String> {
    let path = home_dir()
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db");
    let connection = Connection::open(path).ok()?;
    connection
        .query_row(
            "SELECT access_token FROM account ORDER BY time_updated DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

async fn cursor(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let cookie = if profile == "Personal" {
        std::env::var("BURNRATE_CURSOR_COOKIE")
            .ok()
            .or_else(|| profiles::manual_value(&ProviderId::Cursor))
            .ok_or(LiveError::Missing)?
    } else {
        profile_json(&ProviderId::Cursor, profile)?
            .get("cookie")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(LiveError::Missing)?
    };
    fetch_cursor_with_cookie(
        profile,
        client,
        cookie,
        "https://cursor.com/api/usage-summary",
    )
    .await
}

pub async fn fetch_cursor_with_cookie(
    profile: &str,
    client: &Client,
    cookie: String,
    endpoint: &str,
) -> Result<UsageSnapshot, LiveError> {
    let response = client
        .get(endpoint)
        .header("Accept", "application/json")
        .header("Cookie", cookie)
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    if let Some(error) = response_error(&response) {
        return Err(error);
    }
    let data = response
        .json::<Value>()
        .await
        .map_err(|_| LiveError::Parse)?;
    parse_cursor_usage_response(profile, &data)
}

pub fn parse_cursor_usage_response(
    profile: &str,
    data: &Value,
) -> Result<UsageSnapshot, LiveError> {
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

#[derive(Default)]
pub struct FetchLiveResult {
    snapshots: Vec<UsageSnapshot>,
    stale_keys: HashSet<String>,
}

fn snapshot_key(snapshot: &UsageSnapshot) -> String {
    format!(
        "{}:{}",
        provider_key(&snapshot.provider),
        snapshot.profile_name
    )
}

fn record_provider_result(
    key: &str,
    result: Result<UsageSnapshot, ProviderError>,
    refresh: &mut FetchLiveResult,
) {
    match result {
        Ok(snapshot) => {
            record_success(key);
            refresh.snapshots.push(snapshot);
        }
        Err(ProviderError::RateLimited(delay)) => {
            record_rate_limit(key, delay);
            refresh.stale_keys.insert(key.to_owned());
        }
        Err(_) => {
            record_failure(key);
        }
    }
}

fn mark_backed_off(key: &str, refresh: &mut FetchLiveResult) {
    refresh.stale_keys.insert(key.to_owned());
}

pub fn merge_live_snapshots(
    previous: &[UsageSnapshot],
    refresh: FetchLiveResult,
) -> Vec<UsageSnapshot> {
    let fresh_keys = refresh
        .snapshots
        .iter()
        .map(snapshot_key)
        .collect::<HashSet<_>>();
    let mut merged = refresh.snapshots;
    merged.extend(previous.iter().filter_map(|snapshot| {
        let key = snapshot_key(snapshot);
        if !fresh_keys.contains(&key) && refresh.stale_keys.contains(&key) {
            let mut stale = snapshot.clone();
            stale.status = SnapshotStatus::Stale;
            stale.error_message = None;
            Some(stale)
        } else {
            None
        }
    }));
    merged
}

pub async fn fetch_live() -> FetchLiveResult {
    let client = match Client::builder().user_agent("Burnrate/0.1").build() {
        Ok(client) => client,
        Err(_) => return FetchLiveResult::default(),
    };
    let mut refresh = FetchLiveResult::default();
    let claude_provider = ClaudeProvider {
        client: client.clone(),
    };
    for profile in profiles::list(&ProviderId::Claude) {
        let key = format!("claude:{profile}");
        let available = if profile == "Personal" {
            matches!(claude_provider.detect().await, Detection::Detected)
        } else {
            profiles::credential(&ProviderId::Claude, &profile).is_some()
        };
        if available && can_try(&key) {
            record_provider_result(&key, claude_provider.fetch(&profile).await, &mut refresh);
        } else if available && is_rate_limited(&key) {
            mark_backed_off(&key, &mut refresh);
        }
    }
    let codex_provider = CodexProvider {
        client: client.clone(),
    };
    for profile in profiles::list(&ProviderId::Codex) {
        let key = format!("codex:{profile}");
        let available = if profile == "Personal" {
            matches!(codex_provider.detect().await, Detection::Detected)
        } else {
            profiles::credential(&ProviderId::Codex, &profile).is_some()
        };
        if available && can_try(&key) {
            record_provider_result(&key, codex_provider.fetch(&profile).await, &mut refresh);
        } else if available && is_rate_limited(&key) {
            mark_backed_off(&key, &mut refresh);
        }
    }
    let grok_provider = GrokProvider {
        client: client.clone(),
    };
    for profile in profiles::list(&ProviderId::Grok) {
        let key = format!("grok:{profile}");
        let available = if profile == "Personal" {
            matches!(grok_provider.detect().await, Detection::Detected)
        } else {
            profiles::credential(&ProviderId::Grok, &profile).is_some()
        };
        if available && can_try(&key) {
            record_provider_result(&key, grok_provider.fetch(&profile).await, &mut refresh);
        } else if available && is_rate_limited(&key) {
            mark_backed_off(&key, &mut refresh);
        }
    }
    let opencode_provider = OpencodeProvider {
        client: client.clone(),
    };
    for profile in profiles::list(&ProviderId::Opencode) {
        let key = format!("opencode:{profile}");
        let available = if profile == "Personal" {
            matches!(opencode_provider.detect().await, Detection::Detected)
        } else {
            profiles::credential(&ProviderId::Opencode, &profile).is_some()
        };
        if available && can_try(&key) {
            record_provider_result(&key, opencode_provider.fetch(&profile).await, &mut refresh);
        } else if available && is_rate_limited(&key) {
            mark_backed_off(&key, &mut refresh);
        }
    }
    let cursor_provider = CursorProvider {
        client: client.clone(),
    };
    for profile in profiles::list(&ProviderId::Cursor) {
        let key = format!("cursor:{profile}");
        let available = if profile == "Personal" {
            matches!(cursor_provider.detect().await, Detection::Detected)
        } else {
            profiles::credential(&ProviderId::Cursor, &profile).is_some()
        };
        if available && can_try(&key) {
            record_provider_result(&key, cursor_provider.fetch(&profile).await, &mut refresh);
        } else if available && is_rate_limited(&key) {
            mark_backed_off(&key, &mut refresh);
        }
    }
    refresh
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
            state: if profiles::current_credential(&ProviderId::Cursor).is_some() {
                Detection::Detected
            } else {
                Detection::NotLoggedIn
            },
            profile_name: None,
        },
        DetectedProvider {
            provider: ProviderId::Opencode,
            state: if profiles::current_credential(&ProviderId::Opencode).is_some() {
                Detection::Detected
            } else {
                Detection::NotLoggedIn
            },
            profile_name: None,
        },
    ]
}

pub struct ClaudeProvider {
    pub client: Client,
}
pub struct CodexProvider {
    pub client: Client,
}
pub struct GrokProvider {
    pub client: Client,
}
pub struct CursorProvider {
    pub client: Client,
}
pub struct OpencodeProvider {
    pub client: Client,
}

fn detected_state(provider: ProviderId) -> Detection {
    detected()
        .into_iter()
        .find(|item| item.provider == provider)
        .map(|item| item.state)
        .unwrap_or(Detection::NotLoggedIn)
}

fn provider_error(error: LiveError) -> ProviderError {
    match error {
        LiveError::Missing => ProviderError::NotLoggedIn,
        LiveError::Parse => ProviderError::Parse,
        LiveError::Request => ProviderError::Request,
        LiveError::RateLimited(delay) => ProviderError::RateLimited(delay),
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }
    async fn detect(&self) -> Detection {
        detected_state(self.id())
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        claude(profile, &self.client).await.map_err(provider_error)
    }
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }
    async fn detect(&self) -> Detection {
        detected_state(self.id())
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        codex(profile, &self.client).await.map_err(provider_error)
    }
}

#[async_trait]
impl Provider for GrokProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Grok
    }
    async fn detect(&self) -> Detection {
        detected_state(self.id())
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        grok(profile, &self.client).await.map_err(provider_error)
    }
}

#[async_trait]
impl Provider for CursorProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Cursor
    }
    async fn detect(&self) -> Detection {
        detected_state(self.id())
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        cursor(profile, &self.client).await.map_err(provider_error)
    }
}

#[async_trait]
impl Provider for OpencodeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Opencode
    }
    async fn detect(&self) -> Detection {
        detected_state(self.id())
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        opencode(profile, &self.client)
            .await
            .map_err(provider_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn rate_limited_endpoint(retry_after: Option<&str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind rate-limit endpoint");
        let address = listener.local_addr().expect("read rate-limit address");
        let retry_after = retry_after.map(str::to_owned);
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let retry_header = retry_after
                .map(|value| format!("Retry-After: {value}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\n{retry_header}Content-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://{address}/usage")
    }

    async fn forced_rate_limit(retry_after: Option<&str>) -> LiveError {
        let client = Client::builder()
            .no_proxy()
            .build()
            .expect("build test client");
        let response = client
            .get(rate_limited_endpoint(retry_after))
            .send()
            .await
            .expect("receive forced 429");
        response_error(&response).expect("429 produces an error")
    }

    #[tokio::test]
    async fn retry_after_header_controls_provider_backoff() {
        assert_eq!(
            forced_rate_limit(Some("47")).await,
            LiveError::RateLimited(Duration::from_secs(47))
        );
    }

    #[tokio::test]
    async fn rate_limit_keeps_last_card_values_stale_and_does_not_block_other_providers() {
        let error = forced_rate_limit(None).await;
        assert_eq!(
            error,
            LiveError::RateLimited(DEFAULT_RATE_LIMIT_BACKOFF),
            "429 without Retry-After backs off for at least five minutes"
        );

        let previous_claude = snapshot(
            ProviderId::Claude,
            "429-test",
            Some("Claude Pro".into()),
            vec![window("5h", 42.0, None)],
        );
        let fresh_codex = snapshot(
            ProviderId::Codex,
            "429-test",
            Some("ChatGPT Plus".into()),
            vec![window("5h", 19.0, None)],
        );
        let claude_key = snapshot_key(&previous_claude);
        let codex_key = snapshot_key(&fresh_codex);
        let mut refresh = FetchLiveResult::default();

        record_provider_result(&claude_key, Err(provider_error(error)), &mut refresh);
        record_provider_result(&codex_key, Ok(fresh_codex), &mut refresh);

        assert!(
            !can_try(&claude_key),
            "rate-limited Claude stays backed off"
        );
        assert!(is_rate_limited(&claude_key));
        assert!(can_try(&codex_key), "Claude backoff does not block Codex");
        assert!(!is_rate_limited(&codex_key));

        let merged = merge_live_snapshots(&[previous_claude], refresh);
        let claude = merged
            .iter()
            .find(|item| item.provider == ProviderId::Claude)
            .expect("Claude card remains present");
        let codex = merged
            .iter()
            .find(|item| item.provider == ProviderId::Codex)
            .expect("Codex keeps polling");

        assert!(matches!(claude.status, SnapshotStatus::Stale));
        assert_eq!(claude.windows.len(), 1);
        assert!((claude.windows[0].used_pct - 42.0).abs() < f64::EPSILON);
        assert!(claude.error_message.is_none());
        assert_eq!(
            serde_json::to_value(claude)
                .expect("serialize stale card")
                .get("status")
                .and_then(Value::as_str),
            Some("stale"),
            "the card receives the stale tag instead of error or empty state"
        );
        assert!(matches!(codex.status, SnapshotStatus::Fresh));
        assert!((codex.windows[0].used_pct - 19.0).abs() < f64::EPSILON);
    }
}
