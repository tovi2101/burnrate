use crate::models::{SnapshotStatus, UsageSnapshot};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

const HISTORY_HOURS: i64 = 2;
const MIN_SAMPLE_MINUTES: i64 = 30;
const MIN_PACE_PER_HOUR: f64 = 1.0;

#[derive(Debug, Clone)]
struct Sample {
    at: DateTime<Utc>,
    used_pct: f64,
}

#[derive(Debug, Default)]
struct Series {
    reset: Option<DateTime<Utc>>,
    samples: VecDeque<Sample>,
}

#[derive(Debug, Default)]
pub struct PaceTracker {
    series: HashMap<String, Series>,
}

impl PaceTracker {
    pub fn apply(&mut self, snapshots: &mut [UsageSnapshot], refresh_interval: Duration) {
        for snapshot in snapshots {
            if !matches!(snapshot.status, SnapshotStatus::Fresh) {
                for window in &mut snapshot.windows {
                    window.pace_limit_minutes = None;
                }
                continue;
            }
            for window in &mut snapshot.windows {
                window.pace_limit_minutes = None;
                let key = format!(
                    "{}:{}:{}",
                    snapshot.provider, snapshot.profile_name, window.label
                );
                let series = self.series.entry(key).or_default();
                if series.reset != window.resets_at {
                    series.samples.clear();
                    series.reset = window.resets_at;
                }
                if let Some(last) = series.samples.back() {
                    if snapshot.fetched_at <= last.at {
                        window.pace_limit_minutes = projection(series, window.used_pct);
                        continue;
                    }
                    let maximum_gap = ChronoDuration::from_std(refresh_interval.saturating_mul(3))
                        .unwrap_or_else(|_| ChronoDuration::minutes(15));
                    if snapshot.fetched_at - last.at > maximum_gap {
                        series.samples.clear();
                    }
                }
                series.samples.push_back(Sample {
                    at: snapshot.fetched_at,
                    used_pct: window.used_pct,
                });
                let cutoff = snapshot.fetched_at - ChronoDuration::hours(HISTORY_HOURS);
                while series
                    .samples
                    .front()
                    .is_some_and(|sample| sample.at < cutoff)
                {
                    series.samples.pop_front();
                }
                window.pace_limit_minutes = projection(series, window.used_pct);
            }
        }
    }
}

fn projection(series: &Series, current_pct: f64) -> Option<u64> {
    let first = series.samples.front()?;
    let last = series.samples.back()?;
    let elapsed = last.at - first.at;
    if elapsed < ChronoDuration::minutes(MIN_SAMPLE_MINUTES) {
        return None;
    }
    let hours = elapsed.num_milliseconds() as f64 / 3_600_000.0;
    let pace_per_hour = (last.used_pct - first.used_pct) / hours;
    if pace_per_hour < MIN_PACE_PER_HOUR {
        return None;
    }
    let minutes = ((100.0 - current_pct.max(0.0)) / pace_per_hour * 60.0).max(0.0);
    Some(minutes.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProviderId, UsageWindow};
    use chrono::TimeZone;

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + minutes * 60, 0)
            .single()
            .expect("test timestamp")
    }

    fn sample(minutes: i64, used_pct: f64, reset: i64) -> UsageSnapshot {
        UsageSnapshot {
            provider: ProviderId::Claude,
            profile_name: "Test".into(),
            plan_name: None,
            windows: vec![UsageWindow {
                label: "5h".into(),
                used_pct,
                resets_at: Some(at(reset)),
                pace_limit_minutes: None,
            }],
            fetched_at: at(minutes),
            status: SnapshotStatus::Fresh,
            error_message: None,
        }
    }

    fn apply(tracker: &mut PaceTracker, mut snapshot: UsageSnapshot) -> Option<u64> {
        tracker.apply(
            std::slice::from_mut(&mut snapshot),
            Duration::from_secs(300),
        );
        snapshot.windows[0].pace_limit_minutes
    }

    #[test]
    fn steady_spend_projects_same_window_limit() {
        let mut tracker = PaceTracker::default();
        let mut projected = None;
        for minute in [0, 5, 10, 15, 20, 25, 30] {
            projected = apply(
                &mut tracker,
                sample(minute, 40.0 + minute as f64 / 3.0, 300),
            );
        }
        assert_eq!(projected, Some(150));
    }

    #[test]
    fn idle_series_has_no_projection() {
        let mut tracker = PaceTracker::default();
        let mut projected = None;
        for minute in [0, 5, 10, 15, 20, 25, 30] {
            projected = apply(
                &mut tracker,
                sample(minute, 40.0 + minute as f64 * (0.4 / 30.0), 300),
            );
        }
        assert_eq!(projected, None);
    }

    #[test]
    fn reset_mid_series_discards_old_window_samples() {
        let mut tracker = PaceTracker::default();
        for minute in [0, 5, 10, 15, 20, 25, 30] {
            let _ = apply(
                &mut tracker,
                sample(minute, 40.0 + minute as f64 / 3.0, 300),
            );
        }
        assert_eq!(apply(&mut tracker, sample(31, 2.0, 600)), None);
        let mut projected = None;
        for minute in [36, 41, 46, 51, 56, 61] {
            projected = apply(
                &mut tracker,
                sample(minute, 2.0 + (minute - 31) as f64 / 3.0, 600),
            );
        }
        assert_eq!(projected, Some(264));
    }

    #[test]
    fn app_gap_starts_a_fresh_segment() {
        let mut tracker = PaceTracker::default();
        for minute in [0, 5, 10, 15, 20, 25, 30] {
            let _ = apply(
                &mut tracker,
                sample(minute, 20.0 + minute as f64 / 5.0, 300),
            );
        }
        assert_eq!(apply(&mut tracker, sample(61, 35.0, 300)), None);
        let mut projected = None;
        for minute in [66, 71, 76, 81, 86, 91] {
            projected = apply(
                &mut tracker,
                sample(minute, 35.0 + (minute - 61) as f64 / 5.0, 300),
            );
        }
        assert_eq!(projected, Some(295));
    }
}
