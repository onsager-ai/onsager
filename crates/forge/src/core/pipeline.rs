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
use onsager_spine::bundle::{BundleId, Outputs, SealError, SealRequest, Warehouse};
use onsager_spine::factory_event::{GatePoint, ShapingOutcome, VerdictSummary};
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
    /// Emitted after a successful release seals a new bundle
    /// (warehouse-and-delivery-v0.1 §5.1).
    BundleSealed {
        artifact_id: String,
        bundle_id: BundleId,
        version: u32,
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

/// Synchronous sealing sink — abstracts over the async warehouse for the
/// sync pipeline (warehouse-and-delivery-v0.1 §5.1).
///
/// Production implementations wrap a [`Warehouse`] (async) and block on it
/// inside a `tokio::runtime::Handle::block_on`. Tests use an in-memory mock
/// that returns a deterministic [`BundleId`].
pub trait SealSink: Send + Sync {
    fn seal_release(
        &self,
        artifact_id: &onsager_spine::artifact::ArtifactId,
        result: &ShapingResult,
    ) -> Result<SealedRef, SealError>;
}

/// Pointer to a bundle that a [`SealSink`] just produced.
#[derive(Debug, Clone)]
pub struct SealedRef {
    pub bundle_id: BundleId,
    pub version: u32,
}

/// Blocking adapter turning an async [`Warehouse`] into a sync [`SealSink`].
///
/// The adapter derives a minimal [`Outputs`] from `ShapingResult`: one manifest
/// entry per declared `content_ref`, with the URI as the path and the URI
/// bytes as the blob. Real integrations that need actual file contents will
/// pre-fetch them and supply their own [`SealSink`].
pub struct WarehouseSealSink<W: Warehouse + 'static> {
    warehouse: std::sync::Arc<W>,
    runtime: tokio::runtime::Handle,
}

impl<W: Warehouse + 'static> WarehouseSealSink<W> {
    pub fn new(warehouse: std::sync::Arc<W>, runtime: tokio::runtime::Handle) -> Self {
        Self { warehouse, runtime }
    }
}

impl<W: Warehouse + 'static> SealSink for WarehouseSealSink<W> {
    fn seal_release(
        &self,
        artifact_id: &onsager_spine::artifact::ArtifactId,
        result: &ShapingResult,
    ) -> Result<SealedRef, SealError> {
        let mut outputs = Outputs::new();
        if let Some(content_ref) = &result.content_ref {
            // Minimal placeholder entry. A downstream SealSink that understands
            // the content_ref scheme (git, S3, HTTP) would fetch the real bytes.
            outputs.push(content_ref.uri.clone(), content_ref.uri.as_bytes().to_vec());
        }
        let metadata = serde_json::json!({
            "change_summary": result.change_summary,
            "duration_ms": result.duration_ms,
        });
        let req = SealRequest {
            artifact_id: artifact_id.clone(),
            sealed_by: result.session_id.clone(),
            metadata,
            outputs,
        };
        let warehouse = self.warehouse.clone();
        let runtime = self.runtime.clone();
        // `block_in_place` permits nesting a blocking `block_on` inside an
        // active Tokio runtime without panicking; matches the pattern used by
        // the HTTP sync adapters in `crates/forge/src/cmd/serve.rs`.
        let bundle = tokio::task::block_in_place(|| {
            runtime.block_on(async move { warehouse.seal(req).await })
        })?;
        Ok(SealedRef {
            bundle_id: bundle.bundle_id,
            version: bundle.version,
        })
    }
}

/// The Forge pipeline — orchestrates one tick of the factory loop.
pub struct ForgePipeline<S: StiglabDispatcher, G: SynodicGate> {
    pub store: ArtifactStore,
    pub state: ForgeState,
    stiglab: S,
    synodic: G,
    /// Optional sealing sink. When set, the pipeline seals a bundle on
    /// successful `Released` transitions (warehouse-and-delivery-v0.1 §5.1).
    /// Absent in legacy deployments; seals are skipped in that case.
    warehouse: Option<Box<dyn SealSink>>,
}

impl<S: StiglabDispatcher, G: SynodicGate> ForgePipeline<S, G> {
    pub fn new(stiglab: S, synodic: G) -> Self {
        Self {
            store: ArtifactStore::new(),
            state: ForgeState::new(),
            stiglab,
            synodic,
            warehouse: None,
        }
    }

    /// Attach a [`SealSink`]. Calls to `tick` will seal a bundle on every
    /// successful transition to `Released`.
    pub fn with_warehouse(mut self, warehouse: Box<dyn SealSink>) -> Self {
        self.warehouse = Some(warehouse);
        self
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
        let request_id = ulid::Ulid::new().to_string();
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

        // Short-circuit on unsuccessful outcomes — don't advance state
        // (forge-v0.1 §5.4: Failed/Aborted → artifact stays in previous state).
        if matches!(
            result.outcome,
            ShapingOutcome::Failed | ShapingOutcome::Aborted
        ) {
            output.events.push(PipelineEvent::Error(format!(
                "shaping {:?} for {}: not advancing state",
                result.outcome, decision.artifact_id
            )));
            return output;
        }

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
                payload: serde_json::to_value(&result).expect("ShapingResult must be serializable"),
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

                // Seal before advancing to Released (warehouse-and-delivery-v0.1
                // §5.1: "Released" implies a sealed bundle exists). If sealing
                // fails, abort the transition — the artifact stays in its
                // prior state and a follow-up tick can retry.
                let sealing_release = decision.target_state == ArtifactState::Released
                    && result.outcome == ShapingOutcome::Completed;
                let sealed = if sealing_release {
                    match &self.warehouse {
                        Some(warehouse) => {
                            match warehouse.seal_release(&decision.artifact_id, &result) {
                                Ok(s) => Some(s),
                                Err(e) => {
                                    output.events.push(PipelineEvent::Error(format!(
                                        "warehouse seal failed for {}: {}",
                                        decision.artifact_id, e
                                    )));
                                    return output;
                                }
                            }
                        }
                        None => None,
                    }
                } else {
                    None
                };

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

                        if let Some(sealed) = sealed {
                            self.store
                                .record_bundle(&decision.artifact_id, sealed.bundle_id.clone());
                            output.events.push(PipelineEvent::BundleSealed {
                                artifact_id: decision.artifact_id.to_string(),
                                bundle_id: sealed.bundle_id,
                                version: sealed.version,
                            });
                        }
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

    /// Mock SealSink: returns a deterministic bundle id per artifact, tracks
    /// how many seals were requested.
    struct MockSeal {
        counter: std::sync::atomic::AtomicU32,
    }
    impl MockSeal {
        fn new() -> Self {
            Self {
                counter: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }
    impl SealSink for MockSeal {
        fn seal_release(
            &self,
            artifact_id: &onsager_spine::artifact::ArtifactId,
            _result: &ShapingResult,
        ) -> Result<SealedRef, SealError> {
            let version = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            Ok(SealedRef {
                bundle_id: BundleId::new(format!("bnd_mock_{}_{}", artifact_id.as_str(), version)),
                version,
            })
        }
    }

    /// Kernel that always targets `Released` for any artifact currently in
    /// `UnderReview`. Used to exercise the seal path without building a full
    /// factory loop.
    struct ReleaseKernel;
    impl SchedulingKernel for ReleaseKernel {
        fn decide(&self, world: &WorldState) -> Option<ShapingDecision> {
            let art = world
                .artifacts
                .iter()
                .find(|a| a.state == ArtifactState::UnderReview)?;
            Some(ShapingDecision {
                artifact_id: art.artifact_id.clone(),
                target_version: art.current_version + 1,
                target_state: ArtifactState::Released,
                shaping_intent: serde_json::Value::Null,
                inputs: vec![],
                constraints: vec![],
                deadline: None,
                priority: 100,
            })
        }

        fn observe(&mut self, _event: &onsager_spine::factory_event::FactoryEvent) {}
    }

    #[test]
    fn seal_emits_bundle_sealed_on_release() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow)
            .with_warehouse(Box::new(MockSeal::new()));
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        // Drive to UnderReview via the baseline kernel.
        let baseline = BaselineKernel::new();
        pipeline.tick(&baseline); // Draft -> InProgress
        pipeline.tick(&baseline); // InProgress -> UnderReview
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::UnderReview
        );

        // Now push to Released and seal.
        let output = pipeline.tick(&ReleaseKernel);
        let sealed_event = output.events.iter().find_map(|e| match e {
            PipelineEvent::BundleSealed {
                artifact_id,
                bundle_id,
                version,
            } => Some((artifact_id.clone(), bundle_id.clone(), *version)),
            _ => None,
        });
        let (evt_artifact, evt_bundle, evt_version) =
            sealed_event.expect("BundleSealed event expected on release");

        assert_eq!(evt_artifact, id.to_string());
        assert_eq!(evt_version, 1);

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Released);
        assert_eq!(art.current_bundle_id.as_ref(), Some(&evt_bundle));
        assert_eq!(art.bundle_history.len(), 1);
    }

    /// SealSink that always returns a terminal sealing error.
    struct FailingSeal;
    impl SealSink for FailingSeal {
        fn seal_release(
            &self,
            _artifact_id: &onsager_spine::artifact::ArtifactId,
            _result: &ShapingResult,
        ) -> Result<SealedRef, SealError> {
            Err(SealError::Invalid("mock seal failure".into()))
        }
    }

    #[test]
    fn seal_failure_blocks_release_transition() {
        // warehouse-and-delivery-v0.1 §5.1: Released implies a sealed bundle.
        // If sealing fails, the artifact must not advance to Released.
        let mut pipeline =
            ForgePipeline::new(MockStiglab, MockSynodicAllow).with_warehouse(Box::new(FailingSeal));
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        let baseline = BaselineKernel::new();
        pipeline.tick(&baseline);
        pipeline.tick(&baseline);
        assert_eq!(
            pipeline.store.get(&id).unwrap().state,
            ArtifactState::UnderReview
        );

        let output = pipeline.tick(&ReleaseKernel);
        // No advance, no sealed event.
        let has_advance = output.events.iter().any(|e| {
            matches!(
                e,
                PipelineEvent::ArtifactAdvanced {
                    to_state: ArtifactState::Released,
                    ..
                }
            )
        });
        assert!(
            !has_advance,
            "sealing failure must abort the release transition"
        );
        let has_sealed = output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::BundleSealed { .. }));
        assert!(!has_sealed);

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::UnderReview);
        assert!(art.current_bundle_id.is_none());
    }

    #[test]
    fn no_seal_when_warehouse_absent() {
        let mut pipeline = ForgePipeline::new(MockStiglab, MockSynodicAllow);
        let id = pipeline.store.register(Kind::Code, "svc", "marvin");

        let baseline = BaselineKernel::new();
        pipeline.tick(&baseline);
        pipeline.tick(&baseline);

        let output = pipeline.tick(&ReleaseKernel);
        let has_sealed = output
            .events
            .iter()
            .any(|e| matches!(e, PipelineEvent::BundleSealed { .. }));
        assert!(
            !has_sealed,
            "pipeline without SealSink must not emit BundleSealed"
        );

        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Released);
        assert!(art.current_bundle_id.is_none());
    }

    #[test]
    fn tick_does_not_advance_on_failed_shaping() {
        /// Mock Stiglab that always fails.
        struct MockStiglabFail;
        impl StiglabDispatcher for MockStiglabFail {
            fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
                ShapingResult {
                    request_id: req.request_id.clone(),
                    outcome: ShapingOutcome::Failed,
                    content_ref: None,
                    change_summary: "shaping failed".into(),
                    quality_signals: vec![],
                    session_id: "mock_session".into(),
                    duration_ms: 100,
                    error: Some(onsager_spine::protocol::ErrorDetail {
                        code: "test_failure".into(),
                        message: "mock failure".into(),
                        retriable: Some(true),
                    }),
                }
            }
        }

        let mut pipeline = ForgePipeline::new(MockStiglabFail, MockSynodicAllow);
        let id = pipeline.store.register(Kind::Code, "test-art", "marvin");

        let kernel = BaselineKernel::new();
        pipeline.tick(&kernel);

        // Artifact should remain in Draft — not advanced
        let art = pipeline.store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::Draft);
        assert_eq!(art.current_version, 0);
    }
}
