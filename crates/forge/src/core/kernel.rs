//! Scheduling kernel — the pluggable decision engine (forge-v0.1 §4).
//!
//! The kernel answers one question each tick:
//!   "Given the current world state, what is the next artifact to shape, and how?"
//!
//! v0.1 ships a baseline FIFO + priority kernel. The trait is the contract;
//! implementations are replaceable.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use onsager_spine::artifact::{Artifact, ArtifactState};
use onsager_spine::factory_event::FactoryEvent;
use onsager_spine::protocol::{Insight, ShapingDecision};

/// The world state visible to the scheduling kernel (forge-v0.1 §4.2).
#[derive(Debug, Default)]
pub struct WorldState {
    /// All artifacts in non-terminal states.
    pub artifacts: Vec<Artifact>,
    /// Recent insights from Ising (advisory).
    pub insights: Vec<Insight>,
    /// Number of in-flight shaping requests.
    pub in_flight_count: usize,
    /// Maximum concurrent in-flight requests.
    pub max_in_flight: usize,
}

/// The scheduling kernel contract (forge-v0.1 §4.3).
///
/// Any implementation that honors this interface is valid.
pub trait SchedulingKernel: Send + Sync {
    /// Produce the next shaping decision, or `None` if there is no work.
    fn decide(&self, world: &WorldState) -> Option<ShapingDecision>;

    /// Observe a factory event (for updating internal state).
    fn observe(&mut self, event: &FactoryEvent);
}

// ---------------------------------------------------------------------------
// Baseline kernel: priority queue + FIFO within same priority
// ---------------------------------------------------------------------------

/// A schedulable artifact with its priority.
#[derive(Debug, Clone)]
struct SchedulableArtifact {
    artifact: Artifact,
    priority: i32,
    /// Tie-breaker: earlier creation time goes first (FIFO).
    created_at_ms: i64,
}

impl PartialEq for SchedulableArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.created_at_ms == other.created_at_ms
    }
}
impl Eq for SchedulableArtifact {}

impl PartialOrd for SchedulableArtifact {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchedulableArtifact {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority first, then earlier creation (FIFO).
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.created_at_ms.cmp(&self.created_at_ms))
    }
}

/// Baseline scheduling kernel: priority queue with FIFO tie-breaking.
///
/// - Picks the highest-priority artifact in `Draft` or `InProgress` state
/// - Skips artifacts that are `UnderReview`, `Released`, or `Archived`
/// - Respects `max_in_flight` concurrency limit
#[derive(Debug, Default, Clone)]
pub struct BaselineKernel {
    /// Failure counts per artifact (from Ising insights).
    failure_counts: std::collections::HashMap<String, u32>,
}

impl BaselineKernel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute priority for an artifact. Higher = more urgent.
    fn compute_priority(&self, artifact: &Artifact) -> i32 {
        let base = match artifact.state {
            ArtifactState::Draft => 10,
            ArtifactState::InProgress => 20,
            _ => 0,
        };

        // Deprioritize artifacts with known failure patterns (Ising advisory).
        let failure_penalty = self
            .failure_counts
            .get(artifact.artifact_id.as_str())
            .copied()
            .unwrap_or(0) as i32;

        base - failure_penalty.min(base)
    }
}

impl SchedulingKernel for BaselineKernel {
    fn decide(&self, world: &WorldState) -> Option<ShapingDecision> {
        if world.in_flight_count >= world.max_in_flight {
            return None;
        }

        let mut heap = BinaryHeap::new();

        for artifact in &world.artifacts {
            // Only schedule artifacts that need shaping.
            if !matches!(
                artifact.state,
                ArtifactState::Draft | ArtifactState::InProgress
            ) {
                continue;
            }

            let priority = self.compute_priority(artifact);
            if priority <= 0 {
                continue;
            }

            heap.push(SchedulableArtifact {
                artifact: artifact.clone(),
                priority,
                created_at_ms: artifact.created_at.timestamp_millis(),
            });
        }

        let top = heap.pop()?;
        let target_version = top.artifact.current_version + 1;
        let target_state = match top.artifact.state {
            ArtifactState::Draft => ArtifactState::InProgress,
            ArtifactState::InProgress => ArtifactState::UnderReview,
            _ => return None,
        };

        Some(ShapingDecision {
            artifact_id: top.artifact.artifact_id.clone(),
            target_version,
            target_state,
            shaping_intent: serde_json::json!({
                "action": "shape",
                "kind": top.artifact.kind.to_string(),
            }),
            inputs: vec![],
            constraints: vec![],
            priority: top.priority,
            deadline: None,
        })
    }

    fn observe(&mut self, event: &FactoryEvent) {
        use onsager_spine::factory_event::FactoryEventKind;

        if let FactoryEventKind::ForgeInsightObserved {
            insight_id: _,
            insight_kind,
            scope,
        } = &event.event
        {
            if *insight_kind == onsager_spine::factory_event::InsightKind::Failure {
                if let onsager_spine::factory_event::InsightScope::SpecificArtifact(id) = scope {
                    *self
                        .failure_counts
                        .entry(id.as_str().to_owned())
                        .or_default() += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::artifact::{Artifact, Kind};

    fn make_artifact(name: &str, state: ArtifactState, version: u32) -> Artifact {
        let mut art = Artifact::new(Kind::Code, name, "test-owner", "system", vec![]);
        art.state = state;
        art.current_version = version;
        art
    }

    #[test]
    fn decide_picks_highest_priority() {
        let kernel = BaselineKernel::new();

        let draft = make_artifact("draft-art", ArtifactState::Draft, 0);
        let in_progress = make_artifact("ip-art", ArtifactState::InProgress, 1);

        let world = WorldState {
            artifacts: vec![draft, in_progress.clone()],
            insights: vec![],
            in_flight_count: 0,
            max_in_flight: 5,
        };

        let decision = kernel.decide(&world).unwrap();
        // InProgress has higher base priority (20) than Draft (10)
        assert_eq!(decision.artifact_id, in_progress.artifact_id);
        assert_eq!(decision.target_state, ArtifactState::UnderReview);
    }

    #[test]
    fn decide_returns_none_when_at_capacity() {
        let kernel = BaselineKernel::new();
        let art = make_artifact("art", ArtifactState::Draft, 0);
        let world = WorldState {
            artifacts: vec![art],
            insights: vec![],
            in_flight_count: 5,
            max_in_flight: 5,
        };
        assert!(kernel.decide(&world).is_none());
    }

    #[test]
    fn decide_skips_terminal_states() {
        let kernel = BaselineKernel::new();
        let released = make_artifact("rel", ArtifactState::Released, 3);
        let archived = make_artifact("arch", ArtifactState::Archived, 2);
        let review = make_artifact("rev", ArtifactState::UnderReview, 1);

        let world = WorldState {
            artifacts: vec![released, archived, review],
            insights: vec![],
            in_flight_count: 0,
            max_in_flight: 5,
        };
        assert!(kernel.decide(&world).is_none());
    }

    #[test]
    fn decide_returns_none_for_empty_world() {
        let kernel = BaselineKernel::new();
        let world = WorldState::default();
        assert!(kernel.decide(&world).is_none());
    }

    #[test]
    fn decide_increments_version() {
        let kernel = BaselineKernel::new();
        let art = make_artifact("art", ArtifactState::Draft, 3);
        let world = WorldState {
            artifacts: vec![art],
            insights: vec![],
            in_flight_count: 0,
            max_in_flight: 5,
        };

        let decision = kernel.decide(&world).unwrap();
        assert_eq!(decision.target_version, 4);
    }

    #[test]
    fn observe_deprioritizes_failing_artifacts() {
        use chrono::Utc;
        use onsager_spine::factory_event::{
            FactoryEvent, FactoryEventKind, InsightKind, InsightScope,
        };
        let mut kernel = BaselineKernel::new();

        let failing_art = make_artifact("failing", ArtifactState::Draft, 0);
        let good_art = make_artifact("good", ArtifactState::Draft, 0);

        // Simulate repeated failure insights for the failing artifact
        for _ in 0..15 {
            kernel.observe(&FactoryEvent {
                event: FactoryEventKind::ForgeInsightObserved {
                    insight_id: "ins_1".into(),
                    insight_kind: InsightKind::Failure,
                    scope: InsightScope::SpecificArtifact(failing_art.artifact_id.clone()),
                },
                correlation_id: None,
                causation_id: None,
                actor: "ising".into(),
                timestamp: Utc::now(),
            });
        }

        let world = WorldState {
            artifacts: vec![failing_art, good_art.clone()],
            insights: vec![],
            in_flight_count: 0,
            max_in_flight: 5,
        };

        let decision = kernel.decide(&world).unwrap();
        // Good artifact should be picked over the penalized one
        assert_eq!(decision.artifact_id, good_art.artifact_id);
    }
}
