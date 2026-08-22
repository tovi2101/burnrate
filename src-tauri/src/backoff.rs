use rand::Rng;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FailureBackoff {
    failures: HashMap<String, (u32, Instant, BackoffKind)>,
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
            .map(|(_, until, _)| Instant::now() >= *until)
            .unwrap_or(true)
    }
    pub fn is_rate_limited(&self, provider: &str) -> bool {
        self.failures
            .get(provider)
            .map(|(_, until, kind)| *kind == BackoffKind::RateLimit && Instant::now() < *until)
            .unwrap_or(false)
    }
    pub fn record_failure(&mut self, provider: &str) {
        let count = self
            .failures
            .get(provider)
            .map(|(count, _, _)| *count + 1)
            .unwrap_or(1)
            .min(8);
        let base = 2_u64.saturating_pow(count).min(300);
        let jitter = rand::rng().random_range(0..=base.max(1));
        self.failures.insert(
            provider.into(),
            (
                count,
                Instant::now() + Duration::from_secs(base + jitter),
                BackoffKind::Failure,
            ),
        );
    }
    pub fn record_rate_limit(&mut self, provider: &str, retry_after: Duration) {
        self.failures.insert(
            provider.into(),
            (0, Instant::now() + retry_after, BackoffKind::RateLimit),
        );
    }
    pub fn record_success(&mut self, provider: &str) {
        self.failures.remove(provider);
    }
}
