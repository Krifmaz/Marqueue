//! A short-lived cache so repeated searches don't hit every site again.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Time-limited map from query to answer. Expired entries are dropped
/// lazily, on lookup and when the cache is full.
#[derive(Debug)]
pub struct QueryCache<K, V> {
    ttl: Duration,
    max_entries: usize,
    entries: Mutex<HashMap<K, (Instant, V)>>,
}

impl<K: Eq + Hash + Clone, V: Clone> QueryCache<K, V> {
    /// A zero `ttl` or `max_entries` turns caching off.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.get_at(key, Instant::now())
    }

    pub fn insert(&self, key: K, value: V) {
        self.insert_at(key, value, Instant::now());
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get_at(&self, key: &K, now: Instant) -> Option<V> {
        let mut entries = self.lock();
        let (at, v) = entries.get(key)?;
        if now.duration_since(*at) < self.ttl {
            return Some(v.clone());
        }
        entries.remove(key);
        None
    }

    fn insert_at(&self, key: K, value: V, now: Instant) {
        if self.ttl.is_zero() || self.max_entries == 0 {
            return;
        }
        let mut entries = self.lock();
        if entries.len() >= self.max_entries && !entries.contains_key(&key) {
            entries.retain(|_, (at, _)| now.duration_since(*at) < self.ttl);
            if entries.len() >= self.max_entries {
                // Still full of live entries: drop the oldest.
                let oldest = entries
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    entries.remove(&k);
                }
            }
        }
        entries.insert(key, (now, value));
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<K, (Instant, V)>> {
        // Every write is a single map call, so a poisoned lock is still consistent.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_after_ttl() {
        let c = QueryCache::new(Duration::from_secs(60), 10);
        let t0 = Instant::now();
        c.insert_at("q", 1, t0);
        assert_eq!(c.get_at(&"q", t0 + Duration::from_secs(59)), Some(1));
        assert_eq!(c.get_at(&"q", t0 + Duration::from_secs(60)), None);
        assert!(c.is_empty());
    }

    #[test]
    fn full_cache_drops_oldest() {
        let c = QueryCache::new(Duration::from_secs(60), 2);
        let t0 = Instant::now();
        c.insert_at("a", 1, t0);
        c.insert_at("b", 2, t0 + Duration::from_secs(1));
        c.insert_at("c", 3, t0 + Duration::from_secs(2));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get_at(&"a", t0 + Duration::from_secs(3)), None);
        assert_eq!(c.get_at(&"c", t0 + Duration::from_secs(3)), Some(3));
    }

    #[test]
    fn zero_ttl_disables() {
        let c = QueryCache::new(Duration::ZERO, 10);
        c.insert("a", 1);
        assert!(c.is_empty());
    }
}
