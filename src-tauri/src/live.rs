//! Live provider adapters. Endpoint details intentionally mirror PROVIDERS.md; credentials are
//! read only into memory and are never included in errors or logs.
use crate::backoff::FailureBackoff;
use crate::models::*;
use crate::profiles;
use crate::providers::{Provider, ProviderError};
use async_trait::async_trait;
use chrono::{DateTime, Local, TimeZone, Utc};
use reqwest::{header::RETRY_AFTER, Client, Response, StatusCode};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LiveError {
    #[error("credentials unavailable")]
    Missing,
    #[error("request failed")]
    Request,
    #[error("rate limited")]
    RateLimited(Option<Duration>),
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

fn record_rate_limit(provider: &str, retry_after: Option<Duration>) -> DateTime<Utc> {
    if let Ok(mut state) = BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
    {
        return state.record_rate_limit(provider, retry_after);
    }
    Utc::now() + chrono::Duration::minutes(5)
}

fn rate_limit_retry_at(provider: &str) -> Option<DateTime<Utc>> {
    BACKOFF
        .get_or_init(|| Mutex::new(FailureBackoff::default()))
        .lock()
        .ok()
        .and_then(|state| state.rate_limit_retry_at(provider))
}

fn retry_after(response: &Response) -> Option<Duration> {
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
        pace_limit_minutes: None,
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

fn claude_account_label(identity: &str) -> String {
    static LABELS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    let Ok(mut labels) = LABELS.get_or_init(|| Mutex::new(HashMap::new())).lock() else {
        return "redacted".into();
    };
    let next = labels.len() + 1;
    let label = *labels.entry(identity.to_owned()).or_insert(next);
    format!("account-{label}")
}

fn log_claude_outbound(kind: &str, profile: &str, identity: &str) {
    eprintln!(
        "claude-outbound timestamp={} kind={} profile={} account={}",
        Utc::now().to_rfc3339(),
        kind,
        profile,
        claude_account_label(identity)
    );
}

fn log_claude_response(kind: &str, profile: &str, response: &Response) {
    let retry_seconds = (response.status() == StatusCode::TOO_MANY_REQUESTS)
        .then(|| retry_after(response).map(|delay| delay.as_secs()))
        .flatten();
    eprintln!(
        "claude-response timestamp={} kind={} profile={} status={} retry_after_seconds={}",
        Utc::now().to_rfc3339(),
        kind,
        profile,
        response.status().as_u16(),
        retry_seconds
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );
}

struct PreparedClaude {
    profile: String,
    token: String,
    account_key: String,
    plan: Option<String>,
}

static ACCOUNT_RECENT_ATTEMPTS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static CLAUDE_FETCH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static CODEX_FETCH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static GROK_FETCH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Default)]
struct AccountSingleFlight {
    gates: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    completed: Mutex<HashMap<String, Instant>>,
    #[cfg(debug_assertions)]
    active: Mutex<HashSet<String>>,
}

impl AccountSingleFlight {
    async fn run<F, Fut>(&self, account_key: &str, action: F) -> Result<(), LiveError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), LiveError>>,
    {
        let gate = {
            let mut gates = self.gates.lock().await;
            gates
                .entry(account_key.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = gate.lock().await;
        if self
            .completed
            .lock()
            .ok()
            .and_then(|completed| completed.get(account_key).copied())
            .map(|finished| finished.elapsed() < Duration::from_secs(30))
            .unwrap_or(false)
        {
            return Ok(());
        }
        #[cfg(debug_assertions)]
        {
            let mut active = self.active.lock().expect("refresh overlap guard poisoned");
            assert!(
                active.insert(account_key.to_owned()),
                "two provider refresh delegations overlapped for the same account"
            );
        }
        let result = action().await;
        if let Ok(mut completed) = self.completed.lock() {
            completed.insert(account_key.to_owned(), Instant::now());
        }
        #[cfg(debug_assertions)]
        {
            self.active
                .lock()
                .expect("refresh overlap guard poisoned")
                .remove(account_key);
        }
        result
    }
}

static CLAUDE_REFRESH_SINGLE_FLIGHT: OnceLock<AccountSingleFlight> = OnceLock::new();

fn account_attempt_recent(account_key: &str, minimum_interval: Duration) -> bool {
    ACCOUNT_RECENT_ATTEMPTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|attempts| attempts.get(account_key).copied())
        .map(|attempt| attempt.elapsed() < minimum_interval)
        .unwrap_or(false)
}

fn mark_account_attempt(account_key: &str) {
    if let Ok(mut attempts) = ACCOUNT_RECENT_ATTEMPTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        attempts.insert(account_key.to_owned(), Instant::now());
    }
}

fn group_profiles_by_source(provider: &ProviderId) -> HashMap<String, Vec<String>> {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    if profiles::is_add_pending(provider) {
        return groups;
    }
    for profile in profiles::list(provider) {
        if let Some(source) = profiles::source_key(provider, &profile) {
            groups.entry(source).or_default().push(profile);
        }
    }
    groups
}

async fn delegate_claude_auth(profile: &str) -> Result<(), LiveError> {
    let mut command = Command::new("claude");
    command.args(["auth", "status"]);
    if let Some(root) = profiles::source_root(&ProviderId::Claude, profile) {
        command.env("CLAUDE_CONFIG_DIR", root);
    }
    eprintln!(
        "claude-auth-delegate timestamp={} profile={} command=claude-auth-status",
        Utc::now().to_rfc3339(),
        profile
    );
    let output = timeout(Duration::from_secs(15), command.output())
        .await
        .map_err(|_| LiveError::Request)?
        .map_err(|_| LiveError::Missing)?;
    if !output.status.success() {
        return Err(LiveError::Missing);
    }
    let status: Value = serde_json::from_slice(&output.stdout).map_err(|_| LiveError::Parse)?;
    if status
        .get("loggedIn")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(LiveError::Missing)
    }
}

async fn prepare_claude(profile: &str, account_key: &str) -> Result<PreparedClaude, LiveError> {
    let mut body = profile_json(&ProviderId::Claude, profile)?;
    let mut oauth = body
        .get("claudeAiOauth")
        .cloned()
        .ok_or(LiveError::Missing)?;
    let expired = oauth
        .get("expiresAt")
        .and_then(Value::as_i64)
        .map(|value| value <= Utc::now().timestamp_millis() + 30_000)
        .unwrap_or(false);
    eprintln!(
        "claude-poll timestamp={} profile={} account={} token_refresh_needed={}",
        Utc::now().to_rfc3339(),
        profile,
        claude_account_label(account_key),
        expired
    );
    if expired {
        CLAUDE_REFRESH_SINGLE_FLIGHT
            .get_or_init(AccountSingleFlight::default)
            .run(account_key, || delegate_claude_auth(profile))
            .await?;
        body = profile_json(&ProviderId::Claude, profile)?;
        oauth = body
            .get("claudeAiOauth")
            .cloned()
            .ok_or(LiveError::Missing)?;
        let still_expired = oauth
            .get("expiresAt")
            .and_then(Value::as_i64)
            .map(|value| value <= Utc::now().timestamp_millis() + 30_000)
            .unwrap_or(false);
        if still_expired {
            return Err(LiveError::Missing);
        }
    }
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .ok_or(LiveError::Missing)?
        .to_owned();
    Ok(PreparedClaude {
        profile: profile.to_owned(),
        token,
        account_key: account_key.to_owned(),
        plan: oauth
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

async fn fetch_prepared_claude(
    prepared: &PreparedClaude,
    client: &Client,
) -> Result<UsageSnapshot, LiveError> {
    log_claude_outbound("usage", &prepared.profile, &prepared.account_key);
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(&prepared.token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/2.1.238")
        .send()
        .await
        .map_err(|_| LiveError::Request)?;
    log_claude_response("usage", &prepared.profile, &response);
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
        &prepared.profile,
        prepared.plan.clone(),
        windows,
    ))
}

async fn claude(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    let account_key =
        profiles::source_key(&ProviderId::Claude, profile).ok_or(LiveError::Missing)?;
    let prepared = prepare_claude(profile, &account_key).await?;
    fetch_prepared_claude(&prepared, client).await
}

async fn codex(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    if let Ok(data) = codex_rpc(profile).await {
        return parse_codex_rate_limits(profile, &data);
    }
    let body = profile_json(&ProviderId::Codex, profile)?;
    let tokens = body.get("tokens").cloned().ok_or(LiveError::Missing)?;
    let token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or(LiveError::Missing)?
        .to_owned();
    let account = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let response = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .header("User-Agent", "CodexBar")
        .header("ChatGPT-Account-Id", &account)
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

fn parse_codex_rate_limits(profile: &str, data: &Value) -> Result<UsageSnapshot, LiveError> {
    let limits = data
        .get("rateLimits")
        .or_else(|| data.get("rate_limits"))
        .unwrap_or(data);
    let mut windows = Vec::new();
    for (keys, label) in [
        (["primary", "primary_window"], "5h"),
        (["secondary", "secondary_window"], "Weekly"),
    ] {
        let Some(item) = limits.get(keys[0]).or_else(|| limits.get(keys[1])) else {
            continue;
        };
        let pct = item
            .get("usedPercent")
            .or_else(|| item.get("used_percent"))
            .and_then(Value::as_f64);
        if let Some(pct) = pct {
            let reset = unix(item.get("resetsAt").or_else(|| item.get("reset_at")));
            windows.push(window(label, pct, reset));
        }
    }
    if windows.is_empty() {
        return Err(LiveError::Parse);
    }
    Ok(snapshot(
        ProviderId::Codex,
        profile,
        limits
            .get("planType")
            .or_else(|| limits.get("plan_type"))
            .or_else(|| data.get("planType"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        windows,
    ))
}

async fn codex_rpc(profile: &str) -> Result<Value, LiveError> {
    let mut command = Command::new("codex");
    command
        .arg("app-server")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(root) = profiles::source_root(&ProviderId::Codex, profile) {
        command.env("CODEX_HOME", root);
    }
    let mut child = command.spawn().map_err(|_| LiveError::Missing)?;
    let mut stdin = child.stdin.take().ok_or(LiveError::Request)?;
    let initialize = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"burnrate","version":"0.1.0"}}});
    let initialized = json!({"method":"initialized","params":{}});
    let rate_limits = json!({"id":2,"method":"account/rateLimits/read","params":{}});
    stdin
        .write_all(format!("{}\n{}\n{}\n", initialize, initialized, rate_limits).as_bytes())
        .await
        .map_err(|_| LiveError::Request)?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or(LiveError::Request)?;
    let mut lines = BufReader::new(stdout).lines();
    let result = timeout(Duration::from_secs(12), async {
        while let Some(line) = lines.next_line().await.map_err(|_| LiveError::Request)? {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
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

async fn grok(profile: &str, client: &Client) -> Result<UsageSnapshot, LiveError> {
    if let Ok(data) = grok_rpc(profile).await {
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

async fn grok_rpc(profile: &str) -> Result<Value, LiveError> {
    let mut command = Command::new("grok");
    command
        .args(["agent", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(root) = profiles::source_root(&ProviderId::Grok, profile) {
        command.env("GROK_HOME", root);
    }
    let mut child = command.spawn().map_err(|_| LiveError::Missing)?;
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
    stale_retry_at: HashMap<String, DateTime<Utc>>,
    preserve_keys: HashSet<String>,
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
            let retry_at = record_rate_limit(key, delay);
            refresh.stale_retry_at.insert(key.to_owned(), retry_at);
        }
        Err(_) => {
            record_failure(key);
        }
    }
}

fn mark_backed_off(key: &str, refresh: &mut FetchLiveResult) {
    if let Some(retry_at) = rate_limit_retry_at(key) {
        refresh.stale_retry_at.insert(key.to_owned(), retry_at);
    }
}

fn rate_limit_message(retry_at: DateTime<Utc>) -> String {
    format!(
        "rate limited, retrying at {}",
        retry_at.with_timezone(&Local).format("%H:%M")
    )
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
        if !fresh_keys.contains(&key) {
            if let Some(retry_at) = refresh.stale_retry_at.get(&key) {
                let mut stale = snapshot.clone();
                stale.status = SnapshotStatus::Stale;
                stale.error_message = Some(rate_limit_message(*retry_at));
                return Some(stale);
            }
            if refresh.preserve_keys.contains(&key) {
                return Some(snapshot.clone());
            }
        }
        None
    }));
    merged
}

pub async fn fetch_live(minimum_interval: Duration) -> FetchLiveResult {
    let client = match Client::builder().user_agent("Burnrate/0.1").build() {
        Ok(client) => client,
        Err(_) => return FetchLiveResult::default(),
    };
    let mut refresh = FetchLiveResult::default();
    for provider in [ProviderId::Claude, ProviderId::Codex, ProviderId::Grok] {
        if profiles::is_add_pending(&provider) {
            refresh.preserve_keys.extend(
                profiles::list(&provider)
                    .into_iter()
                    .map(|profile| format!("{}:{profile}", provider_key(&provider))),
            );
        }
    }
    {
        let _claude_guard = CLAUDE_FETCH_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        for (account_key, group) in group_profiles_by_source(&ProviderId::Claude) {
            let Some(profile) = group.first() else {
                continue;
            };
            if profiles::credential(&ProviderId::Claude, profile).is_none() {
                continue;
            }
            let backoff_key = format!("claude-account:{account_key}");
            let profile_keys = group
                .iter()
                .map(|name| format!("claude:{name}"))
                .collect::<Vec<_>>();
            if !can_try(&backoff_key) {
                if let Some(retry_at) = rate_limit_retry_at(&backoff_key) {
                    for profile_key in profile_keys {
                        refresh.stale_retry_at.insert(profile_key, retry_at);
                    }
                } else {
                    refresh.preserve_keys.extend(profile_keys);
                }
                continue;
            }
            #[cfg(debug_assertions)]
            if std::env::var_os("BURNRATE_FORCE_CLAUDE_429").is_some() {
                let retry_at = record_rate_limit(&backoff_key, None);
                for profile_key in profile_keys {
                    refresh.stale_retry_at.insert(profile_key, retry_at);
                }
                continue;
            }
            if account_attempt_recent(&backoff_key, minimum_interval) {
                refresh.preserve_keys.extend(profile_keys);
                continue;
            }
            mark_account_attempt(&backoff_key);
            let result = match prepare_claude(profile, &account_key).await {
                Ok(prepared) => fetch_prepared_claude(&prepared, &client).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(base_snapshot) => {
                    record_success(&backoff_key);
                    for name in group {
                        let mut profile_snapshot = base_snapshot.clone();
                        profile_snapshot.profile_name = name;
                        refresh.snapshots.push(profile_snapshot);
                    }
                }
                Err(LiveError::RateLimited(delay)) => {
                    let retry_at = record_rate_limit(&backoff_key, delay);
                    for profile_key in profile_keys {
                        refresh.stale_retry_at.insert(profile_key, retry_at);
                    }
                }
                Err(_) => record_failure(&backoff_key),
            }
        }
    }
    let _codex_guard = CODEX_FETCH_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let codex_provider = CodexProvider {
        client: client.clone(),
    };
    for (source, group) in group_profiles_by_source(&ProviderId::Codex) {
        let Some(profile) = group.first() else {
            continue;
        };
        let available = profiles::credential(&ProviderId::Codex, profile).is_some()
            || (profile == "Personal"
                && matches!(codex_provider.detect().await, Detection::Detected));
        let account_key = format!("codex-account:{source}");
        let profile_keys = group
            .iter()
            .map(|name| format!("codex:{name}"))
            .collect::<Vec<_>>();
        let recent = account_attempt_recent(&account_key, minimum_interval);
        if available && can_try(&account_key) && !recent {
            mark_account_attempt(&account_key);
            match codex_provider.fetch(profile).await {
                Ok(base_snapshot) => {
                    record_success(&account_key);
                    for name in group {
                        let mut profile_snapshot = base_snapshot.clone();
                        profile_snapshot.profile_name = name;
                        refresh.snapshots.push(profile_snapshot);
                    }
                }
                Err(ProviderError::RateLimited(delay)) => {
                    let retry_at = record_rate_limit(&account_key, delay);
                    for key in profile_keys {
                        refresh.stale_retry_at.insert(key, retry_at);
                    }
                }
                Err(_) => record_failure(&account_key),
            }
        } else if available && recent {
            refresh.preserve_keys.extend(profile_keys);
        } else if available && is_rate_limited(&account_key) {
            if let Some(retry_at) = rate_limit_retry_at(&account_key) {
                for key in profile_keys {
                    refresh.stale_retry_at.insert(key, retry_at);
                }
            }
        }
    }
    drop(_codex_guard);
    let _grok_guard = GROK_FETCH_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let grok_provider = GrokProvider {
        client: client.clone(),
    };
    for (source, group) in group_profiles_by_source(&ProviderId::Grok) {
        let Some(profile) = group.first() else {
            continue;
        };
        let available = profiles::credential(&ProviderId::Grok, profile).is_some()
            || (profile == "Personal"
                && matches!(grok_provider.detect().await, Detection::Detected));
        let account_key = format!("grok-account:{source}");
        let profile_keys = group
            .iter()
            .map(|name| format!("grok:{name}"))
            .collect::<Vec<_>>();
        let recent = account_attempt_recent(&account_key, minimum_interval);
        if available && can_try(&account_key) && !recent {
            mark_account_attempt(&account_key);
            match grok_provider.fetch(profile).await {
                Ok(base_snapshot) => {
                    record_success(&account_key);
                    for name in group {
                        let mut profile_snapshot = base_snapshot.clone();
                        profile_snapshot.profile_name = name;
                        refresh.snapshots.push(profile_snapshot);
                    }
                }
                Err(ProviderError::RateLimited(delay)) => {
                    let retry_at = record_rate_limit(&account_key, delay);
                    for key in profile_keys {
                        refresh.stale_retry_at.insert(key, retry_at);
                    }
                }
                Err(_) => record_failure(&account_key),
            }
        } else if available && recent {
            refresh.preserve_keys.extend(profile_keys);
        } else if available && is_rate_limited(&account_key) {
            if let Some(retry_at) = rate_limit_retry_at(&account_key) {
                for key in profile_keys {
                    refresh.stale_retry_at.insert(key, retry_at);
                }
            }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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
            LiveError::RateLimited(Some(Duration::from_secs(47)))
        );
    }

    #[tokio::test]
    async fn rate_limit_keeps_last_card_values_stale_and_does_not_block_other_providers() {
        let error = forced_rate_limit(None).await;
        assert_eq!(
            error,
            LiveError::RateLimited(None),
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
        assert!(claude
            .error_message
            .as_deref()
            .is_some_and(|message| message.starts_with("rate limited, retrying at ")));
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

    #[tokio::test]
    async fn concurrent_polls_make_exactly_one_refresh_delegation_per_account() {
        let coordinator = Arc::new(AccountSingleFlight::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let poll = |coordinator: Arc<AccountSingleFlight>, calls: Arc<AtomicUsize>| {
            tokio::spawn(async move {
                coordinator
                    .run("claude:file:test", || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok(())
                    })
                    .await
            })
        };
        let first = poll(coordinator.clone(), calls.clone());
        let second = poll(coordinator, calls.clone());
        first
            .await
            .expect("first poll joins")
            .expect("first succeeds");
        second
            .await
            .expect("second poll joins")
            .expect("second succeeds");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "same-account pollers must share one CLI-owned refresh"
        );
    }
}
