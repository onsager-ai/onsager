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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use onsager_spine::protocol::{GateVerdict, ShapingResult};

/// Verdicts pulled off the spine, keyed by `gate_id` (the correlation id
/// Forge stamped on the originating `forge.gate_requested`).
#[derive(Clone, Default)]
pub struct PendingVerdicts {
    inner: Arc<Mutex<HashMap<String, GateVerdict>>>,
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
            .insert(gate_id.to_string(), verdict);
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
#[derive(Clone, Default)]
pub struct PendingShapings {
    inner: Arc<Mutex<HashMap<String, ShapingResult>>>,
}

impl PendingShapings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, request_id: &str, result: ShapingResult) {
        self.inner
            .lock()
            .expect("PendingShapings mutex poisoned")
            .insert(request_id.to_string(), result);
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
}
