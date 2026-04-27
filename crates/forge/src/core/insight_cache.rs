//! Bounded, thread-safe cache of the most recent Ising insights
//! (issue #36 — close the feedback loop).
//!
//! Forge pulls up to `max_len` most-recent insights into `WorldState.insights`
//! on every pipeline tick so the scheduling kernel can take them into
//! account. The cache is push-only with FIFO eviction — once the oldest
//! insight rolls off the queue, it's gone. A downstream consumer that needs
//! an audit trail should read the `ising.insight_emitted` events directly
//! from the spine; this cache is a hot-path view, not a log.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use onsager_spine::protocol::Insight;

/// Default cache capacity. Small on purpose — insights are advisory priors
/// for the scheduler, and stale ones waste kernel cycles.
pub const DEFAULT_INSIGHT_CACHE_CAPACITY: usize = 64;

/// A bounded ring-buffer of insights.
#[derive(Clone)]
pub struct InsightCache {
    inner: Arc<Mutex<InsightCacheInner>>,
}

struct InsightCacheInner {
    buf: VecDeque<Insight>,
    capacity: usize,
}

impl InsightCache {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(InsightCacheInner {
                buf: VecDeque::with_capacity(capacity),
                capacity,
            })),
        }
    }

    /// Push a new insight, evicting the oldest if at capacity.
    pub fn push(&self, insight: Insight) {
        let mut inner = self.inner.lock().expect("insight cache poisoned");
        if inner.buf.len() == inner.capacity {
            inner.buf.pop_front();
        }
        inner.buf.push_back(insight);
    }

    /// Snapshot the cache, newest-first.
    pub fn recent(&self) -> Vec<Insight> {
        let inner = self.inner.lock().expect("insight cache poisoned");
        inner.buf.iter().rev().cloned().collect()
    }

    /// Number of insights currently held.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("insight cache poisoned").buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .expect("insight cache poisoned")
            .buf
            .is_empty()
    }
}

impl Default for InsightCache {
    fn default() -> Self {
        Self::new(DEFAULT_INSIGHT_CACHE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::protocol::FactoryEventRef;
    use onsager_spine::{InsightKind, InsightScope};

    fn make(id: &str) -> Insight {
        Insight {
            insight_id: id.into(),
            kind: InsightKind::Failure,
            scope: InsightScope::Global,
            observation: "x".into(),
            evidence: vec![FactoryEventRef {
                event_id: 1,
                event_type: "forge.gate_verdict".into(),
            }],
            suggested_action: None,
            confidence: 0.7,
        }
    }

    #[test]
    fn newest_first_snapshot() {
        let cache = InsightCache::new(10);
        cache.push(make("a"));
        cache.push(make("b"));
        cache.push(make("c"));

        let snap = cache.recent();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].insight_id, "c");
        assert_eq!(snap[2].insight_id, "a");
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let cache = InsightCache::new(2);
        cache.push(make("a"));
        cache.push(make("b"));
        cache.push(make("c"));

        let snap = cache.recent();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].insight_id, "c");
        assert_eq!(snap[1].insight_id, "b");
    }

    #[test]
    fn clone_shares_backing_store() {
        // The cache is Arc-wrapped; one clone pushes and the other sees it,
        // which is what the serve loop relies on for handing an "insight
        // sink" to the listener task while the pipeline reads the same data.
        let a = InsightCache::new(4);
        let b = a.clone();
        a.push(make("a"));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn zero_capacity_clamped_to_one() {
        // Defensive: a zero-capacity cache would panic on push_back-then-
        // pop_front; clamp rather than crash.
        let cache = InsightCache::new(0);
        cache.push(make("a"));
        cache.push(make("b"));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.recent()[0].insight_id, "b");
    }
}
