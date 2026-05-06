//! Short-TTL cache for live-hydration proxy reads (#170).
//!
//! Reference-only artifact rows store identity + our derived lifecycle but
//! not provider-authored fields (PR/issue title, body, labels, author).
//! Dashboard renders hydrate those by calling GitHub through stiglab's
//! installation tokens. This cache stores completed GitHub responses for a
//! short TTL window so repeated reads can reuse recent payloads and reduce
//! pressure on the App's 5000/hr rate budget. (No in-flight coalescing —
//! two simultaneous misses still hit GitHub twice; a follow-up could add
//! a per-key Notify if that becomes a hotspot.)
//!
//! - **TTL**: env `PORTAL_PROXY_CACHE_TTL_SECS` (sharing the env var name
//!   from #170 even though the cache moved to stiglab — same knob, same
//!   intent). Default 60s. Set to 0 to disable.
//! - **Invalidation**: on TTL expiry, plus targeted `invalidate` /
//!   `invalidate_prefix` calls (used by `backfill_project` to flush a
//!   project's cached lists after writing skeleton rows). The cache is
//!   process-local and ephemeral; staleness is bounded by TTL, not by
//!   webhook coverage. GitHub remains the single source of truth.
//! - **Bounded growth**: every `put` opportunistically sweeps expired
//!   entries so the map can't grow indefinitely under high key cardinality
//!   without ever being read.
//! - **Multi-replica**: each replica has its own cache. Cross-replica
//!   drift up to TTL is acceptable — strictly better than the unbounded
//!   drift of denormalized writes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ENV_TTL: &str = "PORTAL_PROXY_CACHE_TTL_SECS";
const DEFAULT_TTL_SECS: u64 = 60;

struct Entry {
    inserted: Instant,
    payload: serde_json::Value,
}

pub struct ProxyCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

impl ProxyCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn from_env() -> Self {
        let secs = std::env::var(ENV_TTL)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TTL_SECS);
        Self::new(Duration::from_secs(secs))
    }

    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut entries = self.entries.lock().ok()?;
        let expired = entries
            .get(key)
            .map(|e| e.inserted.elapsed() > self.ttl)
            .unwrap_or(true);
        if expired {
            entries.remove(key);
            None
        } else {
            entries.get(key).map(|e| e.payload.clone())
        }
    }

    /// Insert a fresh payload. No-op when TTL is 0. Opportunistically sweeps
    /// expired entries so the map can't grow without bound under high key
    /// cardinality even if specific keys are never read again.
    pub fn put(&self, key: String, payload: serde_json::Value) {
        if self.ttl.is_zero() {
            return;
        }
        if let Ok(mut entries) = self.entries.lock() {
            let ttl = self.ttl;
            entries.retain(|_, e| e.inserted.elapsed() <= ttl);
            entries.insert(
                key,
                Entry {
                    inserted: Instant::now(),
                    payload,
                },
            );
        }
    }

    /// Drop a single key. Used by writers that know an entry is stale.
    pub fn invalidate(&self, key: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }

    /// Drop every key whose name starts with `prefix`. Used after writes
    /// that change a project-scoped artifact set (e.g. `backfill_project`
    /// flushing `issues:{project_id}:*` and `pulls:{project_id}:*`).
    pub fn invalidate_prefix(&self, prefix: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|k, _| !k.starts_with(prefix));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn miss_then_hit_within_ttl() {
        let cache = ProxyCache::new(Duration::from_secs(60));
        assert!(cache.get("k").is_none());
        cache.put("k".into(), json!({"v": 1}));
        assert_eq!(cache.get("k"), Some(json!({"v": 1})));
    }

    #[test]
    fn ttl_zero_is_disabled() {
        let cache = ProxyCache::new(Duration::ZERO);
        cache.put("k".into(), json!({"v": 1}));
        assert!(cache.get("k").is_none());
    }

    #[test]
    fn expired_entries_are_removed() {
        let cache = ProxyCache::new(Duration::from_millis(1));
        cache.put("k".into(), json!(1));
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get("k").is_none());
    }

    #[test]
    fn invalidate_drops_one() {
        let cache = ProxyCache::new(Duration::from_secs(60));
        cache.put("a".into(), json!(1));
        cache.put("b".into(), json!(2));
        cache.invalidate("a");
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b"), Some(json!(2)));
    }

    #[test]
    fn invalidate_prefix_drops_match() {
        let cache = ProxyCache::new(Duration::from_secs(60));
        cache.put("project:abc:issues:open".into(), json!([]));
        cache.put("project:abc:pulls:open".into(), json!([]));
        cache.put("project:def:issues:open".into(), json!([]));
        cache.invalidate_prefix("project:abc:");
        assert!(cache.get("project:abc:issues:open").is_none());
        assert!(cache.get("project:abc:pulls:open").is_none());
        assert!(cache.get("project:def:issues:open").is_some());
    }

    #[test]
    fn put_sweeps_expired_entries() {
        let cache = ProxyCache::new(Duration::from_millis(2));
        cache.put("a".into(), json!(1));
        std::thread::sleep(Duration::from_millis(5));
        cache.put("b".into(), json!(2));
        // After the second put, the expired `a` entry is gone — even though
        // it was never read, so an unbounded map can't accumulate forever.
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b"), Some(json!(2)));
    }
}
