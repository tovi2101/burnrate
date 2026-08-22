use crate::models::{SnapshotStatus, UsageSnapshot};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const HISTORY_VERSION: &str = "v2-local-history-2";
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStatus {
    pub importing: bool,
    pub imported_rows: usize,
    pub message: Option<String>,
}

impl Default for HistoryStatus {
    fn default() -> Self {
        Self {
            importing: true,
            imported_rows: 0,
            message: None,
        }
    }
}

#[derive(Clone)]
pub struct HistoryState {
    pub store: Arc<HistoryStore>,
    pub status: Arc<RwLock<HistoryStatus>>,
}

#[derive(Debug, Clone)]
struct HistoryRow {
    event_key: String,
    timestamp: i64,
    provider: String,
    profile: String,
    kind: String,
    value: f64,
    plan: Option<String>,
    source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub timestamp: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySeries {
    pub provider: String,
    pub kind: String,
    pub unit: String,
    pub points: Vec<HistoryPoint>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub provider: String,
    pub total_tokens: f64,
    pub peak_percent: Option<f64>,
    pub limit_hits: usize,
    pub most_active_day: Option<String>,
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPayload {
    pub range: String,
    pub importing: bool,
    pub imported_rows: usize,
    pub message: Option<String>,
    pub series: Vec<HistorySeries>,
    pub summaries: Vec<HistorySummary>,
}

impl HistoryStore {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let store = Self { path };
        store.initialize()?;
        Ok(store)
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.path).map_err(|error| error.to_string())
    }

    fn initialize(&self) -> Result<(), String> {
        let connection = self.connect()?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 CREATE TABLE IF NOT EXISTS history (
                   id INTEGER PRIMARY KEY,
                   event_key TEXT NOT NULL UNIQUE,
                   timestamp INTEGER NOT NULL,
                   provider TEXT NOT NULL,
                   profile TEXT NOT NULL,
                   kind TEXT NOT NULL,
                   value REAL NOT NULL,
                   plan TEXT,
                   source TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS history_time ON history(timestamp);
                 CREATE INDEX IF NOT EXISTS history_provider_kind ON history(provider, kind);
                 CREATE TABLE IF NOT EXISTS history_meta (
                   key TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );",
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn is_backfilled(&self) -> Result<bool, String> {
        let connection = self.connect()?;
        let value = connection.query_row(
            "SELECT value FROM history_meta WHERE key = 'backfill_version'",
            [],
            |row| row.get::<_, String>(0),
        );
        Ok(value.is_ok_and(|value| value == HISTORY_VERSION))
    }

    fn mark_backfilled(&self) -> Result<(), String> {
        self.connect()?
            .execute(
                "INSERT INTO history_meta(key, value) VALUES('backfill_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [HISTORY_VERSION],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn cursor_is_backfilled(&self) -> Result<bool, String> {
        let connection = self.connect()?;
        Ok(connection
            .query_row(
                "SELECT value FROM history_meta WHERE key = 'cursor_backfill_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .is_ok_and(|value| value == HISTORY_VERSION))
    }

    fn mark_cursor_backfilled(&self) -> Result<(), String> {
        self.connect()?
            .execute(
                "INSERT INTO history_meta(key, value) VALUES('cursor_backfill_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [HISTORY_VERSION],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn insert_rows(&self, rows: &[HistoryRow]) -> Result<usize, String> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let mut inserted = 0;
        {
            let mut statement = transaction
                .prepare(
                    "INSERT OR IGNORE INTO history
                     (event_key, timestamp, provider, profile, kind, value, plan, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(|error| error.to_string())?;
            for row in rows {
                inserted += statement
                    .execute(params![
                        row.event_key,
                        row.timestamp,
                        row.provider,
                        row.profile,
                        row.kind,
                        row.value,
                        row.plan,
                        row.source
                    ])
                    .map_err(|error| error.to_string())?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(inserted)
    }

    pub fn append_live(&self, snapshots: &[UsageSnapshot]) -> Result<usize, String> {
        let rows = snapshots
            .iter()
            .filter(|snapshot| matches!(snapshot.status, SnapshotStatus::Fresh))
            .flat_map(|snapshot| {
                snapshot.windows.iter().map(move |window| HistoryRow {
                    event_key: format!(
                        "live:{}:{}:{}:{}",
                        snapshot.provider,
                        snapshot.profile_name,
                        window.label,
                        snapshot.fetched_at.timestamp_millis()
                    ),
                    timestamp: snapshot.fetched_at.timestamp_millis(),
                    provider: snapshot.provider.to_string(),
                    profile: snapshot.profile_name.clone(),
                    kind: format!("percent:{}", window.label),
                    value: window.used_pct,
                    plan: snapshot.plan_name.clone(),
                    source: "live",
                })
            })
            .collect::<Vec<_>>();
        let inserted = self.insert_rows(&rows)?;
        self.prune()?;
        Ok(inserted)
    }

    fn prune(&self) -> Result<(), String> {
        let cutoff = Utc::now().timestamp_millis() - 180 * DAY_MS;
        self.connect()?
            .execute("DELETE FROM history WHERE timestamp < ?1", [cutoff])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn backfill_local(&self) -> Result<usize, String> {
        if self.is_backfilled()? {
            return Ok(0);
        }
        self.connect()?
            .execute(
                "DELETE FROM history
                 WHERE source = 'backfill'
                   AND provider IN ('claude', 'codex', 'grok', 'opencode')",
                [],
            )
            .map_err(|error| error.to_string())?;
        let home = user_home();
        let mut rows = Vec::new();
        parse_claude(&home.join(".claude").join("projects"), &mut rows);
        parse_codex(&home.join(".codex").join("sessions"), &mut rows);
        parse_grok(&home.join(".grok").join("sessions"), &mut rows);
        parse_opencode(&home, &mut rows);
        let inserted = self.insert_rows(&rows)?;
        self.prune()?;
        self.mark_backfilled()?;
        Ok(inserted)
    }

    async fn backfill_cursor(&self) -> Result<usize, String> {
        if self.cursor_is_backfilled()? {
            return Ok(0);
        }
        let Some(cookie) = crate::profiles::manual_value(&crate::models::ProviderId::Cursor) else {
            return Ok(0);
        };
        let client = reqwest::Client::builder()
            .user_agent(concat!("Burnrate/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| error.to_string())?;
        let start = Utc::now().timestamp_millis() - 180 * DAY_MS;
        let end = Utc::now().timestamp_millis();
        let mut rows = Vec::new();
        let mut pages = Vec::new();
        let mut expected_total = None;
        let mut completed = false;
        for page in 1..=200_usize {
            let response = client
                .post("https://cursor.com/api/dashboard/get-filtered-usage-events")
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .header("Origin", "https://cursor.com")
                .header("Cookie", &cookie)
                .json(&serde_json::json!({
                    "page": page,
                    "pageSize": 1000,
                    "startDate": start.to_string(),
                    "endDate": end.to_string()
                }))
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!(
                    "Cursor history returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let data = response
                .json::<Value>()
                .await
                .map_err(|error| error.to_string())?;
            let events = data
                .get("usageEventsDisplay")
                .and_then(Value::as_array)
                .ok_or_else(|| "Cursor history response omitted usageEventsDisplay".to_string())?;
            if let Some(total) = data.get("totalUsageEventsCount").and_then(flexible_i64) {
                let total = total as usize;
                if expected_total.is_some_and(|expected| expected != total) {
                    return Err("Cursor history total changed during pagination".into());
                }
                expected_total = Some(total);
            }
            if events.is_empty() {
                completed = true;
                break;
            }
            let short_page = events.len() < 1000;
            pages.push(events.clone());
            if short_page {
                completed = true;
                break;
            }
        }
        if !completed {
            return Err("Cursor history pagination reached its safety limit".into());
        }
        let events = reconcile_cursor_pages(&pages, expected_total)?;
        for (index, event) in events.iter().enumerate() {
            let Some(timestamp) = event.get("timestamp").and_then(flexible_i64) else {
                continue;
            };
            let usage = event.get("tokenUsage").unwrap_or(&Value::Null);
            let tokens = flexible_f64(usage.get("inputTokens"))
                + flexible_f64(usage.get("outputTokens"))
                + flexible_f64(usage.get("cacheWriteTokens"))
                + flexible_f64(usage.get("cacheReadTokens"));
            if tokens <= 0.0 {
                continue;
            }
            let model = event
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            rows.push(HistoryRow {
                event_key: format!("cursor:{timestamp}:{model}:{index}"),
                timestamp,
                provider: "cursor".into(),
                profile: "Personal".into(),
                kind: "tokens".into(),
                value: tokens,
                plan: (model != "unknown").then(|| model.to_owned()),
                source: "backfill",
            });
        }
        let inserted = self.insert_rows(&rows)?;
        self.mark_cursor_backfilled()?;
        Ok(inserted)
    }

    pub fn query(&self, range: &str, status: &HistoryStatus) -> Result<HistoryPayload, String> {
        let duration = match range {
            "24h" => DAY_MS,
            "7d" => 7 * DAY_MS,
            "30d" => 30 * DAY_MS,
            _ => return Err("Unsupported history range".into()),
        };
        let bucket_size = if range == "24h" { HOUR_MS } else { DAY_MS };
        let cutoff = Utc::now().timestamp_millis() - duration;
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT timestamp, provider, profile, kind, value
                 FROM history WHERE timestamp >= ?1 ORDER BY timestamp ASC",
            )
            .map_err(|error| error.to_string())?;
        let records = statement
            .query_map([cutoff], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;

        let mut buckets: BTreeMap<(String, String, i64), f64> = BTreeMap::new();
        let mut summaries: BTreeMap<String, HistorySummary> = BTreeMap::new();
        let mut daily_tokens: BTreeMap<(String, i64), f64> = BTreeMap::new();
        let mut limit_state: HashMap<(String, String, String), bool> = HashMap::new();
        for (timestamp, provider, profile, kind, value) in records {
            let bucket = timestamp.div_euclid(bucket_size) * bucket_size;
            let unit = if kind == "tokens" {
                "tokens"
            } else if kind.starts_with("percent:") {
                "percent"
            } else {
                "context_tokens"
            };
            let key = (provider.clone(), kind.clone(), bucket);
            if unit == "percent" {
                buckets
                    .entry(key)
                    .and_modify(|old| *old = old.max(value))
                    .or_insert(value);
            } else {
                *buckets.entry(key).or_default() += value;
            }
            let summary = summaries
                .entry(provider.clone())
                .or_insert_with(|| HistorySummary {
                    provider: provider.clone(),
                    ..HistorySummary::default()
                });
            let date = DateTime::<Utc>::from_timestamp_millis(timestamp)
                .map(|date| date.format("%Y-%m-%d").to_string());
            if summary
                .since
                .as_ref()
                .zip(date.as_ref())
                .is_none_or(|(old, new)| new < old)
            {
                summary.since = date.clone();
            }
            if kind == "tokens" {
                summary.total_tokens += value;
                let day = timestamp.div_euclid(DAY_MS) * DAY_MS;
                *daily_tokens.entry((provider.clone(), day)).or_default() += value;
            } else if kind.starts_with("percent:") {
                summary.peak_percent = Some(summary.peak_percent.unwrap_or_default().max(value));
                let state_key = (provider.clone(), profile, kind);
                let was_at_limit = limit_state.get(&state_key).copied().unwrap_or(false);
                let is_at_limit = value >= 99.0;
                if is_at_limit && !was_at_limit {
                    summary.limit_hits += 1;
                }
                limit_state.insert(state_key, is_at_limit);
            }
        }

        for summary in summaries.values_mut() {
            summary.most_active_day = daily_tokens
                .iter()
                .filter(|((provider, _), _)| provider == &summary.provider)
                .max_by(|left, right| left.1.total_cmp(right.1))
                .and_then(|((_, timestamp), _)| DateTime::<Utc>::from_timestamp_millis(*timestamp))
                .map(|date| date.format("%Y-%m-%d").to_string());
        }

        let mut grouped: BTreeMap<(String, String), Vec<HistoryPoint>> = BTreeMap::new();
        for ((provider, kind, timestamp), value) in buckets {
            grouped
                .entry((provider, kind))
                .or_default()
                .push(HistoryPoint { timestamp, value });
        }
        let series = grouped
            .into_iter()
            .map(|((provider, kind), points)| HistorySeries {
                provider,
                unit: if kind == "tokens" {
                    "tokens"
                } else if kind.starts_with("percent:") {
                    "percent"
                } else {
                    "context_tokens"
                }
                .into(),
                kind,
                points,
            })
            .collect();
        Ok(HistoryPayload {
            range: range.into(),
            importing: status.importing,
            imported_rows: status.imported_rows,
            message: status.message.clone(),
            series,
            summaries: summaries.into_values().collect(),
        })
    }
}

pub fn start_backfill(state: HistoryState) {
    tauri::async_runtime::spawn(async move {
        let store = state.store.clone();
        let result = tauri::async_runtime::spawn_blocking(move || store.backfill_local()).await;
        let local_rows = match result {
            Ok(Ok(rows)) => rows,
            Ok(Err(error)) => {
                let mut status = state.status.write().await;
                status.importing = false;
                status.message = Some(format!("History import incomplete: {error}"));
                return;
            }
            Err(error) => {
                let mut status = state.status.write().await;
                status.importing = false;
                status.message = Some(format!("History import task failed: {error}"));
                return;
            }
        };
        let cursor_result = state.store.backfill_cursor().await;
        let mut status = state.status.write().await;
        match cursor_result {
            Ok(cursor_rows) => {
                let imported_rows = local_rows + cursor_rows;
                status.imported_rows = imported_rows;
                status.message = Some(if imported_rows == 0 {
                    "Local history is up to date".into()
                } else {
                    format!("Imported {imported_rows} local usage records")
                });
            }
            Err(error) => {
                status.imported_rows = local_rows;
                status.message = Some(format!(
                    "Local history imported; Cursor history unavailable: {error}"
                ));
            }
        }
        status.importing = false;
    });
}

pub fn append_live(app: &tauri::AppHandle, snapshots: &[UsageSnapshot]) {
    use tauri::Manager;
    let Some(state) = app.try_state::<HistoryState>() else {
        return;
    };
    if let Err(error) = state.store.append_live(snapshots) {
        eprintln!("history: live sample write failed: {error}");
    }
}

fn user_home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn json_lines(path: &Path) -> Vec<Value> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(if number < 10_000_000_000 {
            number * 1000
        } else {
            number
        });
    }
    value
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|time| time.timestamp_millis())
}

fn number(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or_default()
}

fn flexible_f64(value: Option<&Value>) -> f64 {
    value
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or_default()
}

fn flexible_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|value| *value >= 0)
}

fn reconcile_cursor_pages(
    pages: &[Vec<Value>],
    expected_total: Option<usize>,
) -> Result<Vec<Value>, String> {
    let raw = pages.iter().flatten().cloned().collect::<Vec<_>>();
    let Some(expected) = expected_total else {
        return Ok(raw);
    };
    if raw.len() < expected {
        return Err(format!(
            "Cursor history pagination was incomplete: expected {expected}, received {}",
            raw.len()
        ));
    }
    if raw.len() == expected {
        return Ok(raw);
    }

    let mut removals = raw.len() - expected;
    let mut reconciled = pages.first().cloned().unwrap_or_default();
    for index in 1..pages.len() {
        let previous = &pages[index - 1];
        let current = &pages[index];
        let limit = previous.len().min(current.len());
        let overlap = (1..=limit)
            .rev()
            .find(|count| previous[previous.len() - count..] == current[..*count])
            .unwrap_or_default();
        let remove = overlap.min(removals);
        reconciled.extend(current.iter().skip(remove).cloned());
        removals -= remove;
    }
    if removals != 0 || reconciled.len() != expected {
        return Err(format!(
            "Cursor history pagination was inconsistent: expected {expected}, received {}",
            raw.len()
        ));
    }
    Ok(reconciled)
}

fn opencode_token_total(tokens: &Value) -> f64 {
    let parts = number(tokens.get("input"))
        + number(tokens.get("output"))
        + number(tokens.pointer("/cache/read"))
        + number(tokens.pointer("/cache/write"));
    if parts > 0.0 {
        parts
    } else {
        number(tokens.get("total"))
    }
}

fn parse_claude(root: &Path, rows: &mut Vec<HistoryRow>) {
    let mut seen = HashSet::new();
    for path in files_with_extension(root, "jsonl") {
        for (line_number, record) in json_lines(&path).into_iter().enumerate() {
            let candidate = if record.get("type").and_then(Value::as_str) == Some("progress") {
                record.pointer("/data/message").unwrap_or(&record)
            } else {
                &record
            };
            let message = candidate.get("message").unwrap_or(candidate);
            if message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role != "assistant")
            {
                continue;
            }
            let Some(usage) = message.get("usage") else {
                continue;
            };
            let timestamp = candidate
                .get("timestamp")
                .or_else(|| record.get("timestamp"))
                .and_then(timestamp_ms);
            let Some(timestamp) = timestamp else { continue };
            let message_id = message.get("id").and_then(Value::as_str).unwrap_or("");
            let request_id = candidate
                .get("requestId")
                .or_else(|| record.get("requestId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let dedupe = if message_id.is_empty() && request_id.is_empty() {
                format!("{}:{line_number}", path.display())
            } else {
                format!("{message_id}:{request_id}")
            };
            if !seen.insert(dedupe.clone()) {
                continue;
            }
            let tokens = number(usage.get("input_tokens"))
                + number(usage.get("output_tokens"))
                + number(usage.get("cache_creation_input_tokens"))
                + number(usage.get("cache_read_input_tokens"));
            if tokens <= 0.0 {
                continue;
            }
            rows.push(HistoryRow {
                event_key: format!("claude:{dedupe}"),
                timestamp,
                provider: "claude".into(),
                profile: "Local CLI".into(),
                kind: "tokens".into(),
                value: tokens,
                plan: message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source: "backfill",
            });
        }
    }
}

fn parse_codex(root: &Path, rows: &mut Vec<HistoryRow>) {
    for path in files_with_extension(root, "jsonl") {
        let records = json_lines(&path);
        let mut last_signature = String::new();
        let mut model: Option<String> = None;
        let mut saw_session_meta = false;
        let mut suppressing_fork_copies = false;
        let mut fork_copy_anchor = 0;
        for (line_number, record) in records.into_iter().enumerate() {
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                if saw_session_meta {
                    continue;
                }
                saw_session_meta = true;
                let payload = record.get("payload").unwrap_or(&Value::Null);
                let source_is_subagent = payload
                    .pointer("/source/subagent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
                    .is_some();
                let forked = payload
                    .get("forked_from_id")
                    .and_then(Value::as_str)
                    .is_some()
                    || source_is_subagent;
                if forked {
                    if let Some(timestamp) = record.get("timestamp").and_then(timestamp_ms) {
                        suppressing_fork_copies = true;
                        fork_copy_anchor = timestamp;
                    }
                }
                continue;
            }
            if record.get("type").and_then(Value::as_str) == Some("turn_context") {
                model = record
                    .pointer("/payload/model")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                continue;
            }
            if record.get("type").and_then(Value::as_str) != Some("event_msg")
                || record.pointer("/payload/type").and_then(Value::as_str) != Some("token_count")
            {
                continue;
            }
            let Some(usage) = record.pointer("/payload/info/last_token_usage") else {
                continue;
            };
            let timestamp = record.get("timestamp").and_then(timestamp_ms);
            let Some(timestamp) = timestamp else { continue };
            if model.as_deref().is_none_or(str::is_empty) {
                continue;
            }
            let signature = usage.to_string();
            if signature == last_signature {
                continue;
            }
            last_signature = signature;
            if suppressing_fork_copies {
                if timestamp - fork_copy_anchor < 1000 {
                    fork_copy_anchor = timestamp;
                    continue;
                }
                suppressing_fork_copies = false;
            }
            let input = number(usage.get("input_tokens"));
            let cached = number(usage.get("cached_input_tokens"));
            let cache_write = number(usage.get("cache_write_input_tokens"));
            let output = number(usage.get("output_tokens"));
            let tokens = (input - cached - cache_write).max(0.0) + cached + cache_write + output;
            if tokens <= 0.0 {
                continue;
            }
            rows.push(HistoryRow {
                event_key: format!("codex:{}:{line_number}", path.display()),
                timestamp,
                provider: "codex".into(),
                profile: "Local CLI".into(),
                kind: "tokens".into(),
                value: tokens,
                plan: model.clone(),
                source: "backfill",
            });
        }
    }
}

fn parse_grok(root: &Path, rows: &mut Vec<HistoryRow>) {
    let update_files = files_with_extension(root, "jsonl")
        .into_iter()
        .filter(|path| path.file_name().and_then(|value| value.to_str()) == Some("updates.jsonl"))
        .collect::<Vec<_>>();
    let mut sessions_with_updates = HashSet::new();
    let mut seen = HashSet::new();
    for path in update_files {
        let mut found = false;
        for (line_number, record) in json_lines(&path).into_iter().enumerate() {
            let Some(update) = record.pointer("/params/update") else {
                continue;
            };
            if update.get("sessionUpdate").and_then(Value::as_str) != Some("turn_completed") {
                continue;
            }
            let timestamp = record
                .pointer("/_meta/agentTimestampMs")
                .or_else(|| record.get("timestamp"))
                .and_then(timestamp_ms);
            let Some(timestamp) = timestamp else { continue };
            let Some(usage) = update.get("usage") else {
                continue;
            };
            let event_id = record.pointer("/_meta/eventId").and_then(Value::as_str);
            if let Some(model_usage) = usage.get("modelUsage").and_then(Value::as_object) {
                for (model, model_tokens) in model_usage {
                    let tokens = number(model_tokens.get("inputTokens"))
                        + number(model_tokens.get("outputTokens"));
                    if tokens <= 0.0 {
                        continue;
                    }
                    let dedupe = event_id
                        .map(|id| format!("{id}:{model}"))
                        .unwrap_or_else(|| format!("{}:{line_number}:{model}", path.display()));
                    if !seen.insert(dedupe.clone()) {
                        continue;
                    }
                    found = true;
                    rows.push(HistoryRow {
                        event_key: format!("grok:{dedupe}"),
                        timestamp,
                        provider: "grok".into(),
                        profile: "Local CLI".into(),
                        kind: "tokens".into(),
                        value: tokens,
                        plan: Some(model.clone()),
                        source: "backfill",
                    });
                }
            } else {
                let tokens = number(usage.get("inputTokens")) + number(usage.get("outputTokens"));
                if tokens <= 0.0 {
                    continue;
                }
                let model = record
                    .pointer("/params/model")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let dedupe = event_id
                    .map(|id| format!("{id}:{model}"))
                    .unwrap_or_else(|| format!("{}:{line_number}:{model}", path.display()));
                if !seen.insert(dedupe.clone()) {
                    continue;
                }
                found = true;
                rows.push(HistoryRow {
                    event_key: format!("grok:{dedupe}"),
                    timestamp,
                    provider: "grok".into(),
                    profile: "Local CLI".into(),
                    kind: "tokens".into(),
                    value: tokens,
                    plan: (model != "unknown").then(|| model.to_owned()),
                    source: "backfill",
                });
            }
        }
        if found {
            if let Some(parent) = path.parent() {
                sessions_with_updates.insert(parent.to_path_buf());
            }
        }
    }
    for path in files_with_extension(root, "json")
        .into_iter()
        .filter(|path| path.file_name().and_then(|value| value.to_str()) == Some("signals.json"))
    {
        if path
            .parent()
            .is_some_and(|parent| sessions_with_updates.contains(parent))
        {
            continue;
        }
        let Ok(record) = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .ok_or(())
        else {
            continue;
        };
        let value = number(record.get("contextTokensUsed"))
            .max(number(record.get("totalTokensBeforeCompaction")));
        if value <= 0.0 {
            continue;
        }
        let timestamp = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let Some(timestamp) = timestamp else { continue };
        rows.push(HistoryRow {
            event_key: format!("grok-signal:{}", path.display()),
            timestamp,
            provider: "grok".into(),
            profile: "Local CLI".into(),
            kind: "session_context_tokens".into(),
            value,
            plan: record
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source: "backfill",
        });
    }
}

fn parse_opencode(home: &Path, rows: &mut Vec<HistoryRow>) {
    let root = home.join(".local").join("share").join("opencode");
    let database = root.join("opencode.db");
    let mut seen = HashSet::new();
    if let Ok(connection) = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
    {
        if let Ok(mut statement) = connection.prepare("SELECT id, time_created, data FROM message")
        {
            if let Ok(records) = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            }) {
                for (id, timestamp, data) in records.flatten() {
                    let Ok(record) = serde_json::from_str::<Value>(&data) else {
                        continue;
                    };
                    let Some(tokens) = record.get("tokens") else {
                        continue;
                    };
                    let total = opencode_token_total(tokens);
                    if total <= 0.0 || !seen.insert(id.clone()) {
                        continue;
                    }
                    rows.push(HistoryRow {
                        event_key: format!("opencode:{id}"),
                        timestamp: if timestamp < 10_000_000_000 {
                            timestamp * 1000
                        } else {
                            timestamp
                        },
                        provider: "opencode".into(),
                        profile: "Local CLI".into(),
                        kind: "tokens".into(),
                        value: total,
                        plan: record
                            .get("modelID")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        source: "backfill",
                    });
                }
            }
        }
    }
    let legacy = root.join("storage").join("message");
    for path in files_with_extension(&legacy, "json") {
        let Ok(record) = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .ok_or(())
        else {
            continue;
        };
        let id = record.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || !seen.insert(id.into()) {
            continue;
        }
        let Some(tokens) = record.get("tokens") else {
            continue;
        };
        let total = opencode_token_total(tokens);
        let timestamp = record.pointer("/time/created").and_then(timestamp_ms);
        if total <= 0.0 || timestamp.is_none() {
            continue;
        }
        rows.push(HistoryRow {
            event_key: format!("opencode:{id}"),
            timestamp: timestamp.unwrap_or_default(),
            provider: "opencode".into(),
            profile: "Local CLI".into(),
            kind: "tokens".into(),
            value: total,
            plan: record
                .get("modelID")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source: "backfill",
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("burnrate-history-{name}-{nonce}"));
        fs::create_dir_all(&path).expect("create fixture directory");
        path
    }

    #[test]
    fn timestamp_parser_accepts_seconds_milliseconds_and_rfc3339() {
        assert_eq!(
            timestamp_ms(&Value::from(1_700_000_000)),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            timestamp_ms(&Value::from(1_700_000_000_123_i64)),
            Some(1_700_000_000_123)
        );
        assert_eq!(
            timestamp_ms(&Value::from("2026-08-22T00:00:00Z")),
            Some(1_787_356_800_000)
        );
    }

    #[test]
    fn codex_parser_counts_cache_write_without_losing_tokens() {
        let root = fixture_dir("codex");
        let file = root.join("session.jsonl");
        fs::write(
            &file,
            concat!(
                "{\"timestamp\":\"2026-08-22T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"session\"}}\n",
                "{\"timestamp\":\"2026-08-22T00:00:01Z\",\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-test\"}}\n",
                "{\"timestamp\":\"2026-08-22T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":10,\"cached_input_tokens\":20,\"cache_write_input_tokens\":30,\"output_tokens\":5}}}}\n"
            ),
        )
        .expect("write codex fixture");

        let mut rows = Vec::new();
        parse_codex(&root, &mut rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, 55.0);
        fs::remove_dir_all(root).expect("remove codex fixture");
    }

    #[test]
    fn grok_parser_reads_nested_completed_turn_model_usage() {
        let root = fixture_dir("grok");
        let session = root.join("project").join("session");
        fs::create_dir_all(&session).expect("create grok session");
        fs::write(
            session.join("updates.jsonl"),
            "{\"timestamp\":1750000100,\"params\":{\"sessionId\":\"session\",\"update\":{\"sessionUpdate\":\"turn_completed\",\"usage\":{\"modelUsage\":{\"model-a\":{\"inputTokens\":10,\"outputTokens\":2},\"model-b\":{\"inputTokens\":20,\"outputTokens\":4}}}}},\"_meta\":{\"eventId\":\"event\"}}\n",
        )
        .expect("write grok fixture");

        let mut rows = Vec::new();
        parse_grok(&root, &mut rows);
        rows.sort_by(|left, right| left.plan.cmp(&right.plan));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].value, 12.0);
        assert_eq!(rows[1].value, 24.0);
        fs::remove_dir_all(root).expect("remove grok fixture");
    }

    #[test]
    fn cursor_page_reconciliation_removes_only_proven_boundary_overlap() {
        let pages = vec![
            vec![Value::from(1), Value::from(2)],
            vec![Value::from(2), Value::from(3)],
        ];
        assert_eq!(
            reconcile_cursor_pages(&pages, Some(3)).expect("reconcile pages"),
            vec![Value::from(1), Value::from(2), Value::from(3)]
        );
    }

    #[test]
    fn opencode_parser_falls_back_to_reported_total() {
        let total_only = serde_json::json!({ "total": 123 });
        let parts = serde_json::json!({
            "input": 100,
            "output": 10,
            "cache": { "read": 50, "write": 25 },
            "total": 999
        });
        assert_eq!(opencode_token_total(&total_only), 123.0);
        assert_eq!(opencode_token_total(&parts), 185.0);
    }
}
