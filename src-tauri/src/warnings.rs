use crate::models::{ProviderId, SnapshotStatus, UsageSnapshot, UsageWindow};
use chrono::{DateTime, Local, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningEvent {
    pub body: String,
}

#[derive(Debug, Default)]
pub struct WarningTracker {
    notified: BTreeMap<String, Vec<u8>>,
}

impl WarningTracker {
    pub fn from_persisted(notified: BTreeMap<String, Vec<u8>>) -> Self {
        Self { notified }
    }

    pub fn persisted(&self) -> BTreeMap<String, Vec<u8>> {
        self.notified.clone()
    }

    pub fn evaluate(
        &mut self,
        previous: &[UsageSnapshot],
        current: &[UsageSnapshot],
        enabled: bool,
        thresholds: [u8; 2],
    ) -> Vec<WarningEvent> {
        let mut events = Vec::new();
        for snapshot in current
            .iter()
            .filter(|snapshot| matches!(snapshot.status, SnapshotStatus::Fresh))
        {
            for window in &snapshot.windows {
                let base = window_base(snapshot, window);
                let instance = window_instance(&base, window.resets_at);
                self.notified
                    .retain(|key, _| !key.starts_with(&base) || key == &instance);
                if !enabled {
                    continue;
                }
                let Some(previous_window) = find_previous(previous, snapshot, window) else {
                    continue;
                };
                let fired = self.notified.entry(instance).or_default();
                for threshold in thresholds {
                    if previous_window.used_pct < f64::from(threshold)
                        && window.used_pct >= f64::from(threshold)
                        && !fired.contains(&threshold)
                    {
                        fired.push(threshold);
                        fired.sort_unstable();
                        events.push(WarningEvent {
                            body: warning_body(snapshot, window, threshold),
                        });
                    }
                }
            }
        }
        events
    }
}

fn window_base(snapshot: &UsageSnapshot, window: &UsageWindow) -> String {
    format!("{}\u{1f}{}\u{1f}", snapshot.provider, window.label)
}

fn window_instance(base: &str, reset: Option<DateTime<Utc>>) -> String {
    format!(
        "{base}{}",
        reset
            .map(|value| { format!("five-minute:{}", (value.timestamp() + 150).div_euclid(300)) })
            .unwrap_or_else(|| "no-reset".into())
    )
}

fn find_previous<'a>(
    previous: &'a [UsageSnapshot],
    snapshot: &UsageSnapshot,
    window: &UsageWindow,
) -> Option<&'a UsageWindow> {
    previous
        .iter()
        .find(|candidate| {
            candidate.provider == snapshot.provider
                && candidate.profile_name == snapshot.profile_name
        })?
        .windows
        .iter()
        .find(|candidate| {
            candidate.label == window.label && candidate.resets_at == window.resets_at
        })
}

fn provider_name(provider: &ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Grok => "Grok",
        ProviderId::Cursor => "Cursor",
        ProviderId::Opencode => "OpenCode",
    }
}

fn warning_body(snapshot: &UsageSnapshot, window: &UsageWindow, threshold: u8) -> String {
    let reset = window
        .resets_at
        .map(|value| {
            value
                .with_timezone(&Local)
                .format("%I:%M %p")
                .to_string()
                .trim_start_matches('0')
                .to_owned()
        })
        .unwrap_or_else(|| "unknown".into());
    format!(
        "{} {} window: {}% used, resets {}",
        provider_name(&snapshot.provider),
        window.label,
        threshold,
        reset
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn snapshot(used_pct: f64, reset: i64) -> UsageSnapshot {
        UsageSnapshot {
            provider: ProviderId::Claude,
            profile_name: "Personal".into(),
            plan_name: Some("Max".into()),
            windows: vec![UsageWindow {
                label: "5h".into(),
                used_pct,
                resets_at: Utc.timestamp_opt(reset, 0).single(),
                pace_limit_minutes: None,
            }],
            fetched_at: Utc::now(),
            status: SnapshotStatus::Fresh,
            error_message: None,
        }
    }

    #[test]
    fn threshold_fires_once_and_survives_restart() {
        let mut tracker = WarningTracker::default();
        let events = tracker.evaluate(
            &[snapshot(49.0, 100)],
            &[snapshot(51.0, 100)],
            true,
            [50, 80],
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].body.contains("Claude 5h window: 50% used"));

        let events = tracker.evaluate(
            &[snapshot(51.0, 100)],
            &[snapshot(52.0, 100)],
            true,
            [50, 80],
        );
        assert!(events.is_empty());

        let persisted = tracker.persisted();
        let mut restarted = WarningTracker::from_persisted(persisted);
        let events = restarted.evaluate(
            &[snapshot(49.0, 100)],
            &[snapshot(51.0, 100)],
            true,
            [50, 80],
        );
        assert!(events.is_empty(), "restart must not re-fire an instance");
    }

    #[test]
    fn reset_estimate_jitter_is_the_same_window_instance() {
        let mut tracker = WarningTracker::default();
        assert_eq!(
            tracker
                .evaluate(
                    &[snapshot(49.0, 299)],
                    &[snapshot(51.0, 299)],
                    true,
                    [50, 80]
                )
                .len(),
            1
        );
        let persisted = tracker.persisted();
        let mut restarted = WarningTracker::from_persisted(persisted);
        assert!(restarted
            .evaluate(
                &[snapshot(49.0, 301)],
                &[snapshot(51.0, 301)],
                true,
                [50, 80]
            )
            .is_empty());
    }

    #[test]
    fn reset_rearms_thresholds_without_firing_on_first_sample() {
        let mut tracker = WarningTracker::default();
        assert_eq!(
            tracker
                .evaluate(
                    &[snapshot(49.0, 100)],
                    &[snapshot(51.0, 100)],
                    true,
                    [50, 80]
                )
                .len(),
            1
        );
        assert!(tracker
            .evaluate(
                &[snapshot(51.0, 100)],
                &[snapshot(5.0, 200)],
                true,
                [50, 80]
            )
            .is_empty());
        assert_eq!(
            tracker
                .evaluate(
                    &[snapshot(49.0, 200)],
                    &[snapshot(51.0, 200)],
                    true,
                    [50, 80]
                )
                .len(),
            1
        );
    }

    #[test]
    fn disabled_warnings_never_fire() {
        let mut tracker = WarningTracker::default();
        assert!(tracker
            .evaluate(
                &[snapshot(49.0, 100)],
                &[snapshot(81.0, 100)],
                false,
                [50, 80]
            )
            .is_empty());
    }
}
