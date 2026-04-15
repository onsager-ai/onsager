//! Forge pipeline — the core loop that drives the factory (forge-v0.1 §3).
//!
//! ```text
//! loop:
//!     decision = scheduling_kernel.decide(world_state)
//!     shaping_result = stiglab.dispatch(decision)
//!     verdict = synodic.gate(shaping_result, target_state)
//!     if verdict.allow:
//!         forge.advance(artifact, shaping_result, target_state)
//!         if target_state == RELEASED:
//!             forge.route(artifact, consumers)
//!     emit_factory_events()
//! ```

use onsager_spine::artifact::ArtifactState;
use onsager_spine::factory_event::{GatePoint, VerdictSummary};
use onsager_spine::protocol::{
    GateContext, GateRequest, GateVerdict, ProposedAction, ShapingDecision, ShapingRequest,
    ShapingResult,
};

use super::artifact_store::ArtifactStore;
use super::kernel::{SchedulingKernel, WorldState};
use super::state::ForgeState;

/// Events emitted by a single pipeline tick.
#[derive(Debug, Default)]
pub struct TickOutput {
    /// Factory events produced during this tick.
    pub events: Vec<PipelineEvent>,
}

/// Pipeline-level events (these get translated into FactoryEventKind for the spine).
#[derive(Debug)]
pub enum PipelineEvent {
    DecisionMade(ShapingDecision),
    ShapingDispatched {
        request_id: String,
        artifact_id: String,
        target_version: u32,
    },
    ShapingReturned {
        request_id: String,
        artifact_id: String,
        outcome: String,
    },
    GateRequested {
        artifact_id: String,
        gate_point: GatePoint,
    },
    GateVerdictReceived {
        artifact_id: String,
        gate_point: GatePoint,
        verdict: VerdictSummary,
    },
    ArtifactAdvanced {
        artifact_id: String,
        from_state: ArtifactState,
        to_state: ArtifactState,
    },
    IdleTick,
    Error(String),
}

/// Trait for dispatching shaping work to Stiglab.
///
/// Production implementations call Stiglab's HTTP API.
/// Tests use mock implementations.
pub trait StiglabDispatcher: Send + Sync {
    fn dispatch(&self, request: &ShapingRequest) -> ShapingResult;
}

/// Trait for consulting Synodic at gate points.
///
/// Production implementations call Synodic's gate API.
/// Tests use mock implementations.
pub trait SynodicGate: Send + Sync {
    fn evaluate(&self, request: &GateRequest) -> GateVerdict;
}

/// The Forge pipeline — orchestrates one tick of the factory loop.
pub struct ForgePipeline<S: StiglabDispatcher, G: SynodicGate> {
    pub store: ArtifactStore,
    pub state: ForgeState,
    stiglab: S,
    synodic: G,
    request_counter: u64,
}

impl<S: StiglabDispatcher, G: SynodicGate> ForgePipeline<S, G> {
    pub fn new(stiglab: S, synodic: G) -> Self {
        Self {
            store: ArtifactStore::new(),
            state: ForgeState::new(),
            stiglab,
            synodic,
            request_counter: 0,
        }
    }

    /// Execute one tick of the scheduling loop.
    pub fn tick(&mut self, kernel: &dyn SchedulingKernel) -> TickOutput {
        let mut output = TickOutput::default();

        if !self.state.should_decide() {
            output.events.push(PipelineEvent::IdleTick);
            return output;
        }

        // Build world state.
        let world = WorldState {
            artifacts: self.store.active_artifacts().into_iter().cloned().collect(),
            insights: vec![],
            in_flight_count: 0,
            max_in_flight: 5,
        };

        // Ask the kernel for a decision.
        let decision = match kernel.decide(&world) {
            Some(d) => d,
            None => {
                output.events.push(PipelineEvent::IdleTick);
                return output;
            }
        };

        output
            .events
            .push(PipelineEvent::DecisionMade(decision.clone()));

        // Gate check: pre-dispatch.
        let artifact = match self.store.get(&decision.artifact_id) {
            Some(a) => a.clone(),
            None => {
                output.events.push(PipelineEvent::Error(format!(
                    "artifact {} not found",
                    decision.artifact_id
                )));
                return output;
            }
        };

        let pre_dispatch_gate = GateRequest {
            context: GateContext {
                gate_point: GatePoint::PreDispatch,
                artifact_id: decision.artifact_id.clone(),
                artifact_kind: artifact.kind.clone(),
                current_state: artifact.state,
                target_state: Some(decision.target_state),
                extra: None,
            },
            proposed_action: ProposedAction {
                description: format!("dispatch shaping for {}", decision.artifact_id),
                payload: decision.shaping_intent.clone(),
            },
        };

        output.events.push(PipelineEvent::GateRequested {
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::PreDispatch,
        });

        let verdict = self.synodic.evaluate(&pre_dispatch_gate);
        let verdict_summary = match &verdict {
            GateVerdict::Allow => VerdictSummary::Allow,
            GateVerdict::Deny { .. } => VerdictSummary::Deny,
            GateVerdict::Modify { .. } => VerdictSummary::Modify,
            GateVerdict::Escalate { .. } => VerdictSummary::Escalate,
        };

        output.events.push(PipelineEvent::GateVerdictReceived {
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::PreDispatch,
            verdict: verdict_summary,
        });

        match verdict {
            GateVerdict::Deny { reason } => {
                output.events.push(PipelineEvent::Error(format!(
                    "pre-dispatch gate denied for {}: {}",
                    decision.artifact_id, reason
                )));
                return output;
            }
            GateVerdict::Escalate { .. } => {
                // Park the decision (non-blocking — forge invariant #5).
                return output;
            }
            GateVerdict::Allow | GateVerdict::Modify { .. } => {}
        }

        // Dispatch to Stiglab.
        self.request_counter += 1;
        let request_id = format!("req_{:08x}", self.request_counter);
        let shaping_request = ShapingRequest {
            request_id: request_id.clone(),
            artifact_id: decision.artifact_id.clone(),
            target_version: decision.target_version,
            shaping_intent: decision.shaping_intent.clone(),
            inputs: decision.inputs.clone(),
            constraints: decision.constraints.clone(),
            deadline: decision.deadline,
        };

        output.events.push(PipelineEvent::ShapingDispatched {
            request_id: request_id.clone(),
            artifact_id: decision.artifact_id.to_string(),
            target_version: decision.target_version,
        });

        let result = self.stiglab.dispatch(&shaping_request);

        output.events.push(PipelineEvent::ShapingReturned {
            request_id: request_id.clone(),
            artifact_id: decision.artifact_id.to_string(),
            outcome: format!("{:?}", result.outcome),
        });

        // Gate check: state transition.
        let transition_gate = GateRequest {
            context: GateContext {
                gate_point: GatePoint::StateTransition,
                artifact_id: decision.artifact_id.clone(),
                artifact_kind: artifact.kind.clone(),
                current_state: artifact.state,
                target_state: Some(decision.target_state),
                extra: None,
            },
            proposed_action: ProposedAction {
                description: format!(
                    "advance {} from {} to {}",
                    decision.artifact_id, artifact.state, decision.target_state
                ),
                payload: serde_json::to_value(&result).unwrap_or_default(),
            },
        };

        output.events.push(PipelineEvent::GateRequested {
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::StateTransition,
        });

        let transition_verdict = self.synodic.evaluate(&transition_gate);
        let transition_verdict_summary = match &transition_verdict {
            GateVerdict::Allow => VerdictSummary::Allow,
            GateVerdict::Deny { .. } => VerdictSummary::Deny,
            GateVerdict::Modify { .. } => VerdictSummary::Modify,
            GateVerdict::Escalate { .. } => VerdictSummary::Escalate,
        };

        output.events.push(PipelineEvent::GateVerdictReceived {
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::StateTransition,
            verdict: transition_verdict_summary,
        });

        match transition_verdict {
            GateVerdict::Allow | GateVerdict::Modify { .. } => {
                let from_state = artifact.state;
                match self
                    .store
                    .advance(&decision.artifact_id, decision.target_state, &result)
                {
                    Ok(()) => {
                        output.events.push(PipelineEvent::ArtifactAdvanced {
                            artifact_id: decision.artifact_id.to_string(),
                            from_state,
                            to_state: decision.target_state,
                        });
                    }
                    Err(e) => {
                        output.events.push(PipelineEvent::Error(format!(
                            "failed to advance {}: {}",
                            decision.artifact_id, e
                        )));
                    }
                }
            }
            GateVerdict::Deny { reason } => {
                output.events.push(PipelineEvent::Error(format!(
                    "state transition gate denied for {}: {}",
                    decision.artifact_id, reason
                )));
            }
            GateVerdict::Escalate { .. } => {
                // Park — non-blocking.
            }
        }

        output
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::kernel::BaselineKernel;
    use onsager_spine::artifact::ContentRef;
    use onsager_spine::artifact::Kind;
    use onsager_spine::factory_event::ShapingOutcome;

    /// Mock Stiglab dispatcher that always succeeds.
    struct MockStiglab;
    impl StiglabDispatcher for MockStiglab {
        fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
            ShapingResult {
                request_id: req.request_id.clone(),
                outcome: ShapingOutcome::Completed,
                content_ref: Some(ContentRef {
                    uri: "git://test@abc".into(),
                    checksum: None,
                }),
                change_summary: "mock shaping completed".into(),
                quality_signals: vec![],
                session_id: "mock_session".into(),
                duration_ms: 100,
                error: None,
            }
        }
    }

    /// Mock Synodic gate that always allows.
    struct MockSynodicAllow;
    impl SynodicGate for MockSynodicAllow {
        fn evaluate(&self, _req: &GateRequest) -> GateVerdict {
            GateVerdict::Allow
        }
    }

    /// Mock Synodic gate that always denies.
    struct MockSynodicDeny;
    impl SynodicGate for MockSynodicDeny {
        fn evaluate(&self, _req: &GateRequest) -> GateVerdict {
            GateVerdict::Deny {
                reason: "policy violation".into(),
            }
        }
    }

    #[test]
    fn tick_advances_artifact_when_allowed() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow);
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        let kernel = BaselineKernel::new();
        let output = pipeline.tick(&kernel);

        // Should have: decision, pre-dispatch gate+verdict, dispatch, return,
        // transition gate+verdict, advance
        assert!(output.events.len() >= 7);

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::InProgress);
        assert_eq!(art.current_version, 1);
    }

    #[test]
    fn tick_blocks_when_gate_denies() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicDeny);
        pipeline.store.register(Kind::Code, "test-art", "marvin");

        let kernel = BaselineKernel::new();
        let output = pipeline.tick(&kernel);

        // Should have error event from denied pre-dispatch gate
        let has_error = output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::Error(_)));
        assert!(has_error);
    }

    #[test]
    fn tick_idles_when_no_work() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow);
        // No artifacts registered
        let kernel = BaselineKernel::new();
        let output = pipeline.tick(&kernel);

        assert!(output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::IdleTick)));
    }

    #[test]
    fn tick_idles_when_paused() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow);
        pipeline.store.register(Kind::Code, "test-art", "marvin");

        pipeline
            .state
            .transition(onsager_spine::factory_event::ForgeProcessState::Paused)
            .unwrap();

        let kernel = BaselineKernel::new();
        let output = pipeline.tick(&kernel);

        assert!(output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::IdleTick)));
    }

    #[test]
    fn full_lifecycle_three_ticks() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow);
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        let kernel = BaselineKernel::new();

        // Tick 1: Draft -> InProgress
        pipeline.tick(&kernel);
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::InProgress
        );

        // Tick 2: InProgress -> UnderReview
        pipeline.tick(&kernel);
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::UnderReview
        );

        // Tick 3: UnderReview is not schedulable, so idle
        let output = pipeline.tick(&kernel);
        assert!(output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::IdleTick)));
    }
}
