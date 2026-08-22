use chrono::{DateTime, Utc};
use rand::Rng;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FailureBackoff {
    failures: HashMap<String, BackoffEntry>,
}

struct BackoffEntry {
    count: u32,
    until: Instant,
    retry_at: DateTime<Utc>,
    kind: BackoffKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackoffKind {
    Failure,
    RateLimit,
}

impl FailureBackoff {
    pub fn can_try(&self, provider: &str) -> bool {
        self.failures
            .get(provider)
            .map(|entry| Instant::now() >= entry.until)
            .unwrap_or(true)
    }
    pub fn is_rate_limited(&self, provider: &str) -> bool {
        self.failures
            .get(provider)
            .map(|entry| entry.kind == BackoffKind::RateLimit && Instant::now() < entry.until)
            .unwrap_or(false)
    }
    pub fn rate_limit_retry_at(&self, provider: &str) -> Option<DateTime<Utc>> {
        self.failures.get(provider).and_then(|entry| {
            (entry.kind == BackoffKind::RateLimit && Instant::now() < entry.until)
                .then_some(entry.retry_at)
        })
    }
    pub fn record_failure(&mut self, provider: &str) {
        let count = self
            .failures
            .get(provider)
            .filter(|entry| entry.kind == BackoffKind::Failure)
            .map(|entry| entry.count + 1)
            .unwrap_or(1)
            .min(8);
        let base = 2_u64.saturating_pow(count).min(300);
        let jitter = rand::rng().random_range(0..=base.max(1));
        let delay = Duration::from_secs(base + jitter);
        self.failures.insert(
            provider.into(),
            BackoffEntry {
                count,
                until: Instant::now() + delay,
                retry_at: Utc::now()
                    + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::minutes(5)),
                kind: BackoffKind::Failure,
            },
        );
    }
    pub fn record_rate_limit(
        &mut self,
        provider: &str,
        retry_after: Option<Duration>,
    ) -> DateTime<Utc> {
        let count = self
            .failures
            .get(provider)
            .filter(|entry| entry.kind == BackoffKind::RateLimit)
            .map(|entry| entry.count + 1)
            .unwrap_or(1);
        let fallback_seconds = (5 * 60_u64)
            .saturating_mul(2_u64.saturating_pow(count.saturating_sub(1)))
            .min(30 * 60);
        let delay = retry_after.unwrap_or_else(|| Duration::from_secs(fallback_seconds));
        let retry_at =
            Utc::now() + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::minutes(30));
        self.failures.insert(
            provider.into(),
            BackoffEntry {
                count,
                until: Instant::now() + delay,
                retry_at,
                kind: BackoffKind::RateLimit,
            },
        );
        retry_at
    }
    pub fn record_success(&mut self, provider: &str) {
        self.failures.remove(provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_retry_after_doubles_from_five_to_thirty_minutes() {
        let mut state = FailureBackoff::default();
        let now = Utc::now();
        let first = state.record_rate_limit("claude", None);
        let second = state.record_rate_limit("claude", None);
        let third = state.record_rate_limit("claude", None);
        let fourth = state.record_rate_limit("claude", None);
        assert!((first - now).num_seconds() >= 299);
        assert!((second - now).num_seconds() >= 599);
        assert!((third - now).num_seconds() >= 1199);
        assert!((fourth - now).num_seconds() >= 1799);
        let fifth = state.record_rate_limit("claude", None);
        assert!((fifth - now).num_seconds() < 1802);
    }

    #[test]
    fn retry_after_header_is_preserved_even_above_the_default_ceiling() {
        let mut state = FailureBackoff::default();
        let now = Utc::now();
        let retry_at = state.record_rate_limit("claude", Some(Duration::from_secs(3_600)));
        assert!((retry_at - now).num_seconds() >= 3_599);
    }
}
