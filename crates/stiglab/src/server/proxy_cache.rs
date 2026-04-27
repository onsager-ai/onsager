//! Short-TTL cache for live-hydration proxy reads (#170).
//!
//! Reference-only artifact rows store identity + our derived lifecycle but
//! not provider-authored fields (PR/issue title, body, labels, author).
//! Dashboard renders hydrate those by calling GitHub through stiglab's
//! installation tokens. This cache deduplicates in-flight reads inside a
//! short TTL window so a busy dashboard doesn't burn through the App's
//! 5000/hr rate budget.
//!
//! - **TTL**: env `PORTAL_PROXY_CACHE_TTL_SECS` (sharing the env var name
//!   from #170 even though the cache moved to stiglab — same knob, same
//!   intent). Default 60s. Set to 0 to disable.
//! - **Invalidation**: on TTL expiry only. The cache is process-local and
//!   ephemeral; staleness is bounded by TTL, not webhook coverage. GitHub
//!   remains the single source of truth.
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

    pub fn put(&self, key: String, payload: serde_json::Value) {
        if self.ttl.is_zero() {
            return;
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key,
                Entry {
                    inserted: Instant::now(),
                    payload,
                },
            );
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
}
