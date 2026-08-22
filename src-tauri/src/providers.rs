use crate::models::*;
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("credentials are not available")]
    NotLoggedIn,
    #[error("provider response could not be parsed")]
    Parse,
    #[error("provider request failed")]
    Request,
    #[error("provider rate limited requests for {0:?}")]
    RateLimited(Duration),
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    async fn detect(&self) -> Detection;
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError>;
}

pub struct MockProvider;

impl MockProvider {
    fn window(label: &str, pct: f64, reset: Option<DateTime<Utc>>) -> UsageWindow {
        UsageWindow {
            label: label.to_string(),
            used_pct: pct.clamp(0.0, 100.0),
            resets_at: reset,
        }
    }

    fn parse_date(value: Option<&Value>) -> Option<DateTime<Utc>> {
        let raw = value?.as_str()?;
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|date| date.with_timezone(&Utc))
    }

    fn fixture(path: &str) -> Value {
        let raw = match path {
            "claude" => include_str!("../../fixtures/claude-usage.json"),
            "codex" => include_str!("../../fixtures/codex-usage.json"),
            "grok" => include_str!("../../fixtures/grok-billing.json"),
            _ => "{}",
        };
        serde_json::from_str(raw).unwrap_or_else(|_| Value::Object(Default::default()))
    }

    fn claude(profile: &str) -> UsageSnapshot {
        let body = Self::fixture("claude");
        let five = body.get("five_hour");
        let weekly = body.get("seven_day");
        let mut windows = Vec::new();
        if let Some(value) = five
            .and_then(|v| v.get("utilization"))
            .and_then(Value::as_f64)
        {
            windows.push(Self::window(
                "5h",
                value,
                Self::parse_date(five.and_then(|v| v.get("resets_at"))),
            ));
        }
        if let Some(value) = weekly
            .and_then(|v| v.get("utilization"))
            .and_then(Value::as_f64)
        {
            windows.push(Self::window(
                "Weekly",
                value,
                Self::parse_date(weekly.and_then(|v| v.get("resets_at"))),
            ));
        }
        UsageSnapshot {
            provider: ProviderId::Claude,
            profile_name: profile.to_string(),
            plan_name: Some("Claude Pro".into()),
            windows,
            fetched_at: Utc::now(),
            status: SnapshotStatus::Fresh,
            error_message: None,
        }
    }

    fn codex(profile: &str) -> UsageSnapshot {
        let body = Self::fixture("codex");
        let primary = body.pointer("/rate_limit/primary_window");
        let secondary = body.pointer("/rate_limit/secondary_window");
        let mut windows = Vec::new();
        if let Some(value) = primary
            .and_then(|v| v.get("used_percent"))
            .and_then(Value::as_f64)
        {
            let reset = primary
                .and_then(|v| v.get("reset_at"))
                .and_then(Value::as_i64)
                .and_then(|v| Utc.timestamp_opt(v, 0).single());
            windows.push(Self::window("5h", value, reset));
        }
        if let Some(value) = secondary
            .and_then(|v| v.get("used_percent"))
            .and_then(Value::as_f64)
        {
            let reset = secondary
                .and_then(|v| v.get("reset_at"))
                .and_then(Value::as_i64)
                .and_then(|v| Utc.timestamp_opt(v, 0).single());
            windows.push(Self::window("Weekly", value, reset));
        }
        let plan = body
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        UsageSnapshot {
            provider: ProviderId::Codex,
            profile_name: profile.to_string(),
            plan_name: plan,
            windows,
            fetched_at: Utc::now(),
            status: SnapshotStatus::Fresh,
            error_message: None,
        }
    }

    fn grok(profile: &str) -> UsageSnapshot {
        let body = Self::fixture("grok");
        let config = body.get("config");
        let pct = config
            .and_then(|v| v.get("creditUsagePercent"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let reset = config
            .and_then(|v| v.pointer("/currentPeriod/end"))
            .and_then(|v| Self::parse_date(Some(v)))
            .or_else(|| config.and_then(|v| Self::parse_date(v.get("billingPeriodEnd"))));
        UsageSnapshot {
            provider: ProviderId::Grok,
            profile_name: profile.to_string(),
            plan_name: Some("SuperGrok".into()),
            windows: vec![Self::window("Weekly", pct, reset)],
            fetched_at: Utc::now(),
            status: SnapshotStatus::Fresh,
            error_message: None,
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }
    async fn detect(&self) -> Detection {
        Detection::Detected
    }
    async fn fetch(&self, profile: &str) -> Result<UsageSnapshot, ProviderError> {
        Ok(Self::claude(profile))
    }
}

pub async fn mock_snapshots() -> Vec<UsageSnapshot> {
    vec![
        MockProvider::claude("Personal"),
        MockProvider::codex("Personal"),
        MockProvider::grok("Personal"),
    ]
}
