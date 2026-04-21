//! Gate-completion signal cache (issue #80).
//!
//! Several workflow gate kinds resolve when an external event arrives on the
//! spine. Rather than coupling the stage runner to the spine's async
//! listener, a thin in-memory cache bridges the two: listeners push signals
//! in, the runner pulls matching signals out on each tick.
//!
//! Keyed by `(artifact_id, signal_kind)`, the cache stores the most recent
//! signal under each pair so a burst of the same signal collapses to one
//! observation — the runner only needs to know "has it arrived, and what was
//! the outcome?"

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Outcome of an external signal (e.g. a CI check conclusion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalOutcome {
    /// Success — the gate will resolve as [`super::workflow::GateOutcome::Pass`].
    Success,
    /// Failure with an explanatory reason.
    Failure(String),
}

/// A single observed signal for an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub kind: String,
    pub outcome: SignalOutcome,
}

/// Thread-safe cache shared between signal producers (spine listeners) and
/// the stage runner.
#[derive(Clone, Default)]
pub struct SignalCache {
    inner: Arc<Mutex<HashMap<(String, String), SignalOutcome>>>,
}

impl SignalCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a signal for a given artifact. Overwrites any prior signal
    /// of the same kind for that artifact — most recent wins, matching
    /// GitHub's own check-run re-run semantics.
    pub fn push(&self, artifact_id: &str, signal: Signal) {
        let mut map = self.inner.lock().expect("signal cache poisoned");
        map.insert((artifact_id.to_string(), signal.kind), signal.outcome);
    }

    /// Look up the outcome of a given signal kind for an artifact.
    pub fn get(&self, artifact_id: &str, kind: &str) -> Option<SignalOutcome> {
        let map = self.inner.lock().expect("signal cache poisoned");
        map.get(&(artifact_id.to_string(), kind.to_string()))
            .cloned()
    }

    /// Clear the signal — used when an artifact advances past the stage
    /// that was waiting on it, so a re-entry (revise cycle) starts clean.
    pub fn clear(&self, artifact_id: &str, kind: &str) {
        let mut map = self.inner.lock().expect("signal cache poisoned");
        map.remove(&(artifact_id.to_string(), kind.to_string()));
    }

    /// Clear every signal for an artifact (e.g. on archive).
    pub fn clear_artifact(&self, artifact_id: &str) {
        let mut map = self.inner.lock().expect("signal cache poisoned");
        map.retain(|(a, _), _| a != artifact_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_signals() {
        let cache = SignalCache::new();
        cache.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(cache.get("art_1", "ci"), Some(SignalOutcome::Success));
        assert_eq!(cache.get("art_1", "merge"), None);
        assert_eq!(cache.get("art_2", "ci"), None);
    }

    #[test]
    fn most_recent_wins() {
        let cache = SignalCache::new();
        cache.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Failure("red".into()),
            },
        );
        cache.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(cache.get("art_1", "ci"), Some(SignalOutcome::Success));
    }

    #[test]
    fn clear_removes_individual_signal() {
        let cache = SignalCache::new();
        cache.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );
        cache.push(
            "art_1",
            Signal {
                kind: "merge".into(),
                outcome: SignalOutcome::Success,
            },
        );

        cache.clear("art_1", "ci");
        assert_eq!(cache.get("art_1", "ci"), None);
        assert_eq!(cache.get("art_1", "merge"), Some(SignalOutcome::Success));
    }

    #[test]
    fn clear_artifact_drops_all_signals() {
        let cache = SignalCache::new();
        cache.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );
        cache.push(
            "art_2",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );

        cache.clear_artifact("art_1");
        assert_eq!(cache.get("art_1", "ci"), None);
        assert_eq!(cache.get("art_2", "ci"), Some(SignalOutcome::Success));
    }

    #[test]
    fn clone_shares_backing_store() {
        let a = SignalCache::new();
        let b = a.clone();
        a.push(
            "art_1",
            Signal {
                kind: "ci".into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(b.get("art_1", "ci"), Some(SignalOutcome::Success));
    }
}
