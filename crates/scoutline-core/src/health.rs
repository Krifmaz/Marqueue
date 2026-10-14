//! Per-indexer health: failure history, stats and backoff.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, TimeDelta, Utc};

use crate::error::IndexerError;
use crate::indexer::IndexerId;

/// First backoff after a failure; doubles with each failure in a row.
const BASE_BACKOFF: TimeDelta = TimeDelta::seconds(60);
const MAX_BACKOFF: TimeDelta = TimeDelta::hours(3);
/// Failures in a row before backoff starts; one blip doesn't count.
const FAILURES_BEFORE_BACKOFF: u32 = 2;
/// How many recent failures each indexer keeps.
const HISTORY_LEN: usize = 20;

/// One recorded failure.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub at: DateTime<Utc>,
    pub reason: String,
}

/// Health and stats for one indexer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexerHealth {
    pub queries: u64,
    pub failures: u64,
    pub consecutive_failures: u32,
    pub last_success: Option<DateTime<Utc>>,
    /// Newest last.
    pub history: Vec<Failure>,
    /// Searches skip this indexer until then.
    pub backoff_until: Option<DateTime<Utc>>,
    /// Running average over successful queries.
    pub avg_response_ms: u64,
}

impl IndexerHealth {
    pub fn is_healthy(&self) -> bool {
        self.consecutive_failures == 0
    }

    pub fn last_failure(&self) -> Option<&Failure> {
        self.history.last()
    }
}

/// Tracks health for every indexer. Cheap to share behind an `Arc`.
#[derive(Debug, Default)]
pub struct HealthTracker {
    inner: Mutex<HashMap<IndexerId, IndexerHealth>>,
}

impl HealthTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the backoff error if `id` must be skipped right now.
    pub fn check(&self, id: &str) -> Result<(), IndexerError> {
        self.check_at(id, Utc::now())
    }

    pub fn record_success(&self, id: &str, elapsed_ms: u64) {
        self.record_success_at(id, elapsed_ms, Utc::now());
    }

    pub fn record_failure(&self, id: &str, error: &IndexerError) {
        self.record_failure_at(id, error, Utc::now());
    }

    pub fn get(&self, id: &str) -> Option<IndexerHealth> {
        self.lock().get(id).cloned()
    }

    pub fn snapshot(&self) -> HashMap<IndexerId, IndexerHealth> {
        self.lock().clone()
    }

    /// Clears backoff so the indexer is tried on the next search.
    pub fn reset(&self, id: &str) {
        if let Some(h) = self.lock().get_mut(id) {
            h.consecutive_failures = 0;
            h.backoff_until = None;
        }
    }

    fn check_at(&self, id: &str, now: DateTime<Utc>) -> Result<(), IndexerError> {
        let map = self.lock();
        match map.get(id) {
            Some(h) if h.backoff_until.is_some_and(|until| until > now) => {
                Err(IndexerError::BackingOff {
                    until: h.backoff_until.unwrap(),
                    reason: h
                        .last_failure()
                        .map(|f| f.reason.clone())
                        .unwrap_or_default(),
                })
            }
            _ => Ok(()),
        }
    }

    fn record_success_at(&self, id: &str, elapsed_ms: u64, now: DateTime<Utc>) {
        let mut map = self.lock();
        let h = map.entry(id.to_owned()).or_default();
        h.queries += 1;
        let successes = h.queries - h.failures;
        h.avg_response_ms = (h.avg_response_ms * (successes - 1) + elapsed_ms) / successes;
        h.consecutive_failures = 0;
        h.backoff_until = None;
        h.last_success = Some(now);
    }

    fn record_failure_at(&self, id: &str, error: &IndexerError, now: DateTime<Utc>) {
        if !error.counts_as_failure() {
            return;
        }
        let mut map = self.lock();
        let h = map.entry(id.to_owned()).or_default();
        h.queries += 1;
        h.failures += 1;
        h.consecutive_failures += 1;
        h.history.push(Failure {
            at: now,
            reason: error.to_string(),
        });
        if h.history.len() > HISTORY_LEN {
            h.history.remove(0);
        }
        if h.consecutive_failures >= FAILURES_BEFORE_BACKOFF {
            let doublings = (h.consecutive_failures - FAILURES_BEFORE_BACKOFF).min(16);
            let delay = (BASE_BACKOFF * 2i32.pow(doublings)).min(MAX_BACKOFF);
            h.backoff_until = Some(now + delay);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<IndexerId, IndexerHealth>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err() -> IndexerError {
        IndexerError::Status(503)
    }

    #[test]
    fn backs_off_after_repeated_failures() {
        let t = HealthTracker::new();
        let now = Utc::now();
        t.record_failure_at("a", &err(), now);
        assert!(t.check_at("a", now).is_ok(), "one failure is a blip");

        t.record_failure_at("a", &err(), now);
        assert!(t.check_at("a", now + TimeDelta::seconds(59)).is_err());
        assert!(t.check_at("a", now + TimeDelta::seconds(60)).is_ok());

        t.record_failure_at("a", &err(), now);
        assert_eq!(
            t.get("a").unwrap().backoff_until,
            Some(now + TimeDelta::seconds(120))
        );
    }

    #[test]
    fn backoff_is_capped() {
        let t = HealthTracker::new();
        let now = Utc::now();
        for _ in 0..40 {
            t.record_failure_at("a", &err(), now);
        }
        let h = t.get("a").unwrap();
        assert_eq!(h.backoff_until, Some(now + MAX_BACKOFF));
        assert_eq!(h.history.len(), HISTORY_LEN);
        assert_eq!(h.failures, 40);
    }

    #[test]
    fn success_clears_backoff_and_tracks_speed() {
        let t = HealthTracker::new();
        let now = Utc::now();
        t.record_failure_at("a", &err(), now);
        t.record_failure_at("a", &err(), now);
        t.record_success_at("a", 100, now);
        t.record_success_at("a", 300, now);
        let h = t.get("a").unwrap();
        assert!(h.is_healthy());
        assert_eq!(h.backoff_until, None);
        assert_eq!(h.avg_response_ms, 200);
        assert_eq!(h.queries, 4);
    }
}
