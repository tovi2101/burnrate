use rand::Rng;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FailureBackoff {
    failures: HashMap<String, (u32, Instant)>,
}

impl FailureBackoff {
    pub fn can_try(&self, provider: &str) -> bool {
        self.failures
            .get(provider)
            .map(|(_, until)| Instant::now() >= *until)
            .unwrap_or(true)
    }
    pub fn record_failure(&mut self, provider: &str) {
        let count = self
            .failures
            .get(provider)
            .map(|(count, _)| *count + 1)
            .unwrap_or(1)
            .min(8);
        let base = 2_u64.saturating_pow(count).min(300);
        let jitter = rand::rng().random_range(0..=base.max(1));
        self.failures.insert(
            provider.into(),
            (count, Instant::now() + Duration::from_secs(base + jitter)),
        );
    }
    pub fn record_success(&mut self, provider: &str) {
        self.failures.remove(provider);
    }
}
