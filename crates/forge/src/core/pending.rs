//! In-memory parking maps for non-blocking pipeline decisions
//! (spec #131 / ADR 0004 Lever C, phase 3).
//!
//! Forge can no longer block on a synchronous HTTP roundtrip to ask Synodic
//! for a verdict or wait on Stiglab to finish a shaping. Instead it emits an
//! event (`forge.gate_requested`, `forge.shaping_dispatched`), parks the
//! pipeline decision keyed by the request's correlation id, and the
//! corresponding listener thread records the response into one of these
//! maps when the matching event lands on the spine
//! (`synodic.gate_verdict`, `stiglab.shaping_result_ready`).
//!
//! Phase-4 wires these maps into `ForgePipeline::tick`'s state machine so
//! the resume path consults them. Phase-3 only populates them — keeping
//! the change surface narrow.
//!
//! ## Persistence
//!
//! These maps are process-private and lost on restart. The phase-6 sub-issue
//! covers a forge-private `pending_pipeline_decisions` table + replay so a
//! mid-tick crash doesn't drop in-flight decisions; until then, a restart
//! mid-decision stalls the artifact (the next tick re-emits the request,
//! which Synodic / Stiglab observe as a duplicate keyed by the same
//! correlation id).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use onsager_spine::protocol::{GateVerdict, ShapingResult};

/// Cap on parked entries per map. Each tick of phase 4's resume path
/// drains exactly one entry per artifact, so steady-state size is
/// bounded by the in-flight artifact count. The cap is a safety net
/// for two failure modes:
///
/// 1. Producer bugs that emit verdicts/results for ids forge never
///    parked (no consumer ever claims, entry leaks).
/// 2. Phase-3 → phase-4 partial deploys: forge runs phase-3 binary
///    (parks but never drains) for hours before the phase-4 binary
///    rolls out.
///
/// 4096 fits a multi-day burst at typical factory volumes while
/// keeping the worst-case footprint negligible. Eviction is FIFO on
/// the order entries were inserted — a verdict that has been parked
/// longer than 4096 newer arrivals is almost certainly orphaned and
/// safe to drop. Phase 6 replaces this with persisted parking.
const MAX_PENDING_ENTRIES: usize = 4096;

/// Inner store backing both [`PendingVerdicts`] and [`PendingShapings`].
/// Provides FIFO eviction once `max_entries` is exceeded so the maps
/// can't grow without bound under the failure modes documented above.
struct BoundedMap<V> {
    inner: HashMap<String, V>,
    /// Insertion order, used for FIFO eviction on overflow. Re-inserts
    /// of an existing key reset its position.
    order: VecDeque<String>,
    max_entries: usize,
}

impl<V> BoundedMap<V> {
    fn new() -> Self {
        Self {
            inner: HashMap::new(),
            order: VecDeque::new(),
            max_entries: MAX_PENDING_ENTRIES,
        }
    }

    fn insert(&mut self, key: &str, value: V) {
        // Re-insert: drop the old position so the new one lands at the
        // back of the queue.
        if self.inner.contains_key(key) {
            self.order.retain(|existing| existing != key);
        }
        self.order.push_back(key.to_string());
        self.inner.insert(key.to_string(), value);
        while self.inner.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                if self.inner.remove(&oldest).is_some() {
                    tracing::warn!(
                        evicted_key = %oldest,
                        "forge: pending map at capacity, evicted oldest entry"
                    );
                }
            } else {
                break;
            }
        }
    }

    fn remove(&mut self, key: &str) -> Option<V> {
        let removed = self.inner.remove(key);
        if removed.is_some() {
            self.order.retain(|existing| existing != key);
        }
        removed
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Verdicts pulled off the spine, keyed by `gate_id` (the correlation id
/// Forge stamped on the originating `forge.gate_requested`).
///
/// Bounded by [`MAX_PENDING_ENTRIES`] with FIFO eviction; see module doc.
#[derive(Clone)]
pub struct PendingVerdicts {
    inner: Arc<Mutex<BoundedMap<GateVerdict>>>,
}

impl Default for PendingVerdicts {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BoundedMap::new())),
        }
    }
}

impl PendingVerdicts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a verdict that the pipeline can claim later. Overwrites any
    /// prior entry for the same id — Synodic shouldn't emit twice, but if
    /// it does the latest wins (matches the dashboard's "most recent
    /// verdict" rendering).
    pub fn insert(&self, gate_id: &str, verdict: GateVerdict) {
        self.inner
            .lock()
            .expect("PendingVerdicts mutex poisoned")
            .insert(gate_id, verdict);
    }

    /// Claim the verdict for `gate_id`, removing it from the map. Returns
    /// `None` if no verdict has landed yet.
    pub fn take(&self, gate_id: &str) -> Option<GateVerdict> {
        self.inner
            .lock()
            .expect("PendingVerdicts mutex poisoned")
            .remove(gate_id)
    }

    /// Number of parked verdicts. Used by tests; in production we'd want
    /// a metric here once phase 4 lands the consumer side.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("PendingVerdicts mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Shaping results pulled off the spine, keyed by `request_id` (the id
/// Forge stamped on the originating `forge.shaping_dispatched`).
///
/// Bounded by [`MAX_PENDING_ENTRIES`] with FIFO eviction; see module doc.
#[derive(Clone)]
pub struct PendingShapings {
    inner: Arc<Mutex<BoundedMap<ShapingResult>>>,
}

impl Default for PendingShapings {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BoundedMap::new())),
        }
    }
}

impl PendingShapings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, request_id: &str, result: ShapingResult) {
        self.inner
            .lock()
            .expect("PendingShapings mutex poisoned")
            .insert(request_id, result);
    }

    pub fn take(&self, request_id: &str) -> Option<ShapingResult> {
        self.inner
            .lock()
            .expect("PendingShapings mutex poisoned")
            .remove(request_id)
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("PendingShapings mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::protocol::ShapingResult;

    fn allow() -> GateVerdict {
        GateVerdict::Allow
    }

    fn shaping_ok(req: &str) -> ShapingResult {
        ShapingResult {
            request_id: req.into(),
            outcome: onsager_spine::factory_event::ShapingOutcome::Completed,
            content_ref: None,
            change_summary: String::new(),
            quality_signals: vec![],
            session_id: "sess".into(),
            duration_ms: 0,
            error: None,
        }
    }

    #[test]
    fn pending_verdicts_round_trip() {
        let m = PendingVerdicts::new();
        assert!(m.is_empty());
        m.insert("g1", allow());
        assert_eq!(m.len(), 1);
        let taken = m.take("g1").expect("verdict was parked");
        assert!(matches!(taken, GateVerdict::Allow));
        // Take is single-shot.
        assert!(m.take("g1").is_none());
    }

    #[test]
    fn pending_verdicts_overwrites_on_duplicate_insert() {
        let m = PendingVerdicts::new();
        m.insert("g1", allow());
        m.insert(
            "g1",
            GateVerdict::Deny {
                reason: "later".into(),
            },
        );
        match m.take("g1").unwrap() {
            GateVerdict::Deny { reason } => assert_eq!(reason, "later"),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn pending_shapings_round_trip() {
        let m = PendingShapings::new();
        m.insert("r1", shaping_ok("r1"));
        let taken = m.take("r1").expect("result was parked");
        assert_eq!(taken.request_id, "r1");
        assert!(m.take("r1").is_none());
    }

    #[test]
    fn pending_maps_are_independent_per_key() {
        // Park two distinct requests; taking one must not affect the other.
        let m = PendingShapings::new();
        m.insert("r1", shaping_ok("r1"));
        m.insert("r2", shaping_ok("r2"));
        assert_eq!(m.len(), 2);
        m.take("r1");
        assert_eq!(m.len(), 1);
        assert!(m.take("r2").is_some());
        assert!(m.is_empty());
    }

    #[test]
    fn bounded_map_evicts_oldest_on_overflow() {
        // Producer-bug guard: keep inserting orphaned verdicts and the
        // map size must stay capped, not climb without bound. The
        // oldest entry is the first one to go.
        let mut m = BoundedMap::<u32>::new();
        m.max_entries = 3;

        m.insert("a", 1);
        m.insert("b", 2);
        m.insert("c", 3);
        assert_eq!(m.len(), 3);

        // Overflow → "a" evicted (FIFO).
        m.insert("d", 4);
        assert_eq!(m.len(), 3);
        assert!(m.remove("a").is_none());
        assert_eq!(m.remove("b"), Some(2));
        assert_eq!(m.remove("c"), Some(3));
        assert_eq!(m.remove("d"), Some(4));
    }

    #[test]
    fn bounded_map_reinsert_resets_position() {
        // Re-inserting an existing key moves it to the back of the
        // queue, so it survives the next eviction wave.
        let mut m = BoundedMap::<u32>::new();
        m.max_entries = 2;

        m.insert("a", 1);
        m.insert("b", 2);
        // Touch "a" — it's now newer than "b".
        m.insert("a", 11);
        // Insert "c" — evicts "b" (the oldest), not "a".
        m.insert("c", 3);
        assert!(m.remove("b").is_none());
        assert_eq!(m.remove("a"), Some(11));
        assert_eq!(m.remove("c"), Some(3));
    }
}
