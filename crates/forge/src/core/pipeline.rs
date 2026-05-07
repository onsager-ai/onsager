//! Forge pipeline — the core loop that drives the factory (forge-v0.1 §3).
//!
//! Phase 4 of Lever C (spec #131 / ADR 0004 — see #148) replaced the
//! synchronous loop with a non-blocking parked state machine: each
//! decision emits a request event (`forge.gate_requested`,
//! `forge.shaping_dispatched`), parks itself keyed by the request's
//! correlation id, and resumes on a later tick once the corresponding
//! response (`synodic.gate_verdict`, `stiglab.shaping_result_ready`)
//! lands in the [`PendingVerdicts`] / [`PendingShapings`] maps the
//! event listeners populate.
//!
//! ```text
//! tick:
//!     if parked:
//!         resume_parked()        // drain pending_* maps; advance one stage
//!     else if scheduling_kernel.decide(world_state):
//!         start_decision()       // emit pre-dispatch gate, park
//! ```
//!
//! The pipeline owns at most one parked decision (the legacy
//! one-decision-per-tick model). A future iteration can lift this to
//! per-artifact concurrency by promoting `parked: Option<...>` to
//! `parked: HashMap<ArtifactId, ParkedDecision>`.

use onsager_artifact::{ArtifactState, ArtifactVersionId};
use onsager_spine::factory_event::{GatePoint, ShapingOutcome, VerdictSummary};
use onsager_spine::protocol::{
    GateContext, GateRequest, GateVerdict, ProposedAction, ShapingDecision, ShapingRequest,
    ShapingResult,
};
use onsager_warehouse::{Outputs, SealError, SealRequest, Warehouse};

use onsager_artifact::Artifact;

use super::artifact_store::ArtifactStore;
use super::insight_cache::InsightCache;
use super::kernel::{SchedulingKernel, WorldState};
use super::pending::{PendingShapings, PendingVerdicts};
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
    /// A shaping request was sent to Stiglab. Carries the full
    /// `ShapingRequest` payload so the spine emitter can populate the
    /// `request` field on `forge.shaping_dispatched` (spec #131 / ADR
    /// 0004 Lever C, phase 3) — Stiglab's listener spawns a session
    /// directly off the event without a follow-up HTTP roundtrip.
    ShapingDispatched {
        request_id: String,
        artifact_id: String,
        target_version: u32,
        request: ShapingRequest,
    },
    ShapingReturned {
        request_id: String,
        artifact_id: String,
        outcome: String,
    },
    /// A gate request was sent to Synodic. Carries the full
    /// `GateRequest` payload + the `gate_id` correlation key so the
    /// spine emitter can populate `forge.gate_requested` and the
    /// `synodic.gate_verdict` listener can resume the parked
    /// pipeline decision keyed on the same id (phase 3).
    GateRequested {
        gate_id: String,
        artifact_id: String,
        gate_point: GatePoint,
        request: GateRequest,
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
        bundle_id: ArtifactVersionId,
        version: u32,
    },
    IdleTick,
    Error(String),
}

/// Synchronous sealing sink — abstracts over the async warehouse for the
/// sync pipeline (warehouse-and-delivery-v0.1 §5.1).
///
/// Production implementations wrap a [`Warehouse`] (async) and block on it
/// inside a `tokio::runtime::Handle::block_on`. Tests use an in-memory mock
/// that returns a deterministic [`ArtifactVersionId`].
pub trait SealSink: Send + Sync {
    fn seal_release(
        &self,
        artifact_id: &onsager_artifact::ArtifactId,
        result: &ShapingResult,
    ) -> Result<SealedRef, SealError>;
}

/// Pointer to a bundle that a [`SealSink`] just produced.
#[derive(Debug, Clone)]
pub struct SealedRef {
    pub bundle_id: ArtifactVersionId,
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
        artifact_id: &onsager_artifact::ArtifactId,
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

/// A decision the pipeline kicked off but hasn't finished. The
/// pipeline parks one of these per tick when it emits a request event;
/// the next tick consults the parking maps and advances the stage.
///
/// Process-private and lost on restart. Phase 6 (#186) moves this into
/// a forge-private `pending_pipeline_decisions` table with replay on
/// boot so a mid-tick crash doesn't drop in-flight decisions.
///
/// `pub(crate)` (not `pub`): the parking lifecycle is an internal
/// detail of the pipeline state machine — the public surface is the
/// observable `PipelineEvent` stream + `is_parked()`. Keeping these
/// types crate-private leaves the parking shape free to evolve
/// (per-artifact concurrency, persistence) without breaking external
/// callers.
#[derive(Debug, Clone)]
pub(crate) struct ParkedDecision {
    pub(crate) decision: ShapingDecision,
    /// Snapshot of the artifact at the moment the decision was made.
    /// Held so the pipeline can build follow-up gate requests with the
    /// same `current_state` / `artifact_kind` it used initially —
    /// reading the live store mid-decision could surface a state
    /// changed by an out-of-band write.
    pub(crate) artifact_snapshot: Artifact,
    pub(crate) stage: ParkStage,
}

/// Where a parked decision is in the lifecycle. Each variant carries
/// the correlation id the resume path uses to claim the matching
/// response from the parking maps. The `result` payload on the
/// transition variant is `Box`ed so the enum's stack footprint stays
/// small (the other variants are short strings).
///
/// `pub(crate)` for the same reason as [`ParkedDecision`]. The
/// `Awaiting*` shared prefix mirrors the lifecycle vocabulary used in
/// the module-level docstring and the resume helpers; `clippy`'s
/// uniform-prefix lint is not worth the legibility cost here.
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum ParkStage {
    AwaitingPreDispatchVerdict {
        gate_id: String,
    },
    AwaitingShapingResult {
        request_id: String,
    },
    AwaitingTransitionVerdict {
        gate_id: String,
        result: Box<ShapingResult>,
    },
}

/// The Forge pipeline — orchestrates one tick of the factory loop.
pub struct ForgePipeline {
    pub store: ArtifactStore,
    pub state: ForgeState,
    /// Optional sealing sink. When set, the pipeline seals a bundle on
    /// successful `Released` transitions (warehouse-and-delivery-v0.1 §5.1).
    /// Absent in legacy deployments; seals are skipped in that case.
    warehouse: Option<Box<dyn SealSink>>,
    /// Shared cache of the most recent Ising insights (issue #36). The cache
    /// is an `Arc<Mutex<...>>` so the ising listener can push to it without
    /// holding the pipeline lock. Always present — the default cache is
    /// empty, which preserves the pre-issue-#36 behavior.
    insights: InsightCache,
    /// At-most-one in-flight decision (legacy one-per-tick model).
    /// Phase 4: the resume path drains parking maps keyed by the
    /// correlation ids stored in [`ParkStage`].
    parked: Option<ParkedDecision>,
    /// Verdicts the `gate_verdict_listener` parks for the pipeline to
    /// claim on resume. Cloned in from `ForgeSharedState` so the
    /// listener task and the pipeline see the same map.
    pending_verdicts: PendingVerdicts,
    /// Shaping results the `shaping_result_listener` parks. Same
    /// sharing model as [`Self::pending_verdicts`].
    pending_shapings: PendingShapings,
}

impl ForgePipeline {
    pub fn new(pending_verdicts: PendingVerdicts, pending_shapings: PendingShapings) -> Self {
        Self {
            store: ArtifactStore::new(),
            state: ForgeState::new(),
            warehouse: None,
            insights: InsightCache::default(),
            parked: None,
            pending_verdicts,
            pending_shapings,
        }
    }

    /// Whether the pipeline is currently parked on a decision. Test
    /// helper: gated on `cfg(test)` because no production code reads
    /// it today. If a future dashboard endpoint wants to surface
    /// "in-flight" state, lift the gate to `pub(crate)`.
    #[cfg(test)]
    fn is_parked(&self) -> bool {
        self.parked.is_some()
    }

    /// Attach a [`SealSink`]. Calls to `tick` will seal a bundle on every
    /// successful transition to `Released`.
    pub fn with_warehouse(mut self, warehouse: Box<dyn SealSink>) -> Self {
        self.warehouse = Some(warehouse);
        self
    }

    /// Attach a shared [`InsightCache`]. Hand the same clone to the
    /// ising-event listener so insights flow into `WorldState.insights`.
    pub fn with_insight_cache(mut self, insights: InsightCache) -> Self {
        self.insights = insights;
        self
    }

    /// Clone of the shared insight cache — used by the serve binary to hand
    /// the same backing store to the ising event listener.
    pub fn insight_cache(&self) -> InsightCache {
        self.insights.clone()
    }

    /// Execute one tick of the scheduling loop.
    ///
    /// Two branches: drain a parked decision if one is in-flight, or
    /// (if the process state allows scheduling) ask the kernel for a
    /// fresh decision and park it. Both paths emit `PipelineEvent`s
    /// describing what the tick observed; the caller in `cmd/serve.rs`
    /// translates those into spine events.
    pub fn tick(&mut self, kernel: &dyn SchedulingKernel) -> TickOutput {
        let mut output = TickOutput::default();

        if !self.state.should_decide() {
            output.events.push(PipelineEvent::IdleTick);
            return output;
        }

        if self.parked.is_some() {
            // A decision is in-flight; try to advance it. If the
            // matching response hasn't arrived yet, the resume path
            // emits `IdleTick` and leaves the parked state intact.
            self.resume_parked(&mut output);
            return output;
        }

        let world = WorldState {
            artifacts: self.store.active_artifacts().into_iter().cloned().collect(),
            insights: self.insights.recent(),
            in_flight_count: 0,
            max_in_flight: 5,
        };

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

        self.start_decision(decision, artifact, &mut output);
        output
    }

    /// Begin a new decision: emit the pre-dispatch gate request and
    /// park. The `synodic.gate_verdict` listener resolves the parked
    /// decision on a future tick once Synodic emits the matching verdict.
    fn start_decision(
        &mut self,
        decision: ShapingDecision,
        artifact_snapshot: Artifact,
        output: &mut TickOutput,
    ) {
        let gate_id = ulid::Ulid::new().to_string();
        let gate_request = build_pre_dispatch_gate(&decision, &artifact_snapshot);

        output.events.push(PipelineEvent::GateRequested {
            gate_id: gate_id.clone(),
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::PreDispatch,
            request: gate_request,
        });

        self.parked = Some(ParkedDecision {
            decision,
            artifact_snapshot,
            stage: ParkStage::AwaitingPreDispatchVerdict { gate_id },
        });
    }
}

impl ForgePipeline {
    /// Resume a parked decision: claim the matching response from the
    /// parking maps and advance one stage. If no response has arrived,
    /// re-park unchanged and emit `IdleTick` so the caller can
    /// distinguish "waiting" from "decided but did nothing".
    fn resume_parked(&mut self, output: &mut TickOutput) {
        let Some(parked) = self.parked.take() else {
            output.events.push(PipelineEvent::IdleTick);
            return;
        };

        match parked.stage {
            ParkStage::AwaitingPreDispatchVerdict { ref gate_id } => {
                let Some(verdict) = self.pending_verdicts.take(gate_id) else {
                    self.parked = Some(parked);
                    output.events.push(PipelineEvent::IdleTick);
                    return;
                };
                self.advance_after_pre_dispatch(parked, verdict, output);
            }
            ParkStage::AwaitingShapingResult { ref request_id } => {
                let Some(result) = self.pending_shapings.take(request_id) else {
                    self.parked = Some(parked);
                    output.events.push(PipelineEvent::IdleTick);
                    return;
                };
                self.advance_after_shaping(parked, result, output);
            }
            ParkStage::AwaitingTransitionVerdict { ref gate_id, .. } => {
                let Some(verdict) = self.pending_verdicts.take(gate_id) else {
                    self.parked = Some(parked);
                    output.events.push(PipelineEvent::IdleTick);
                    return;
                };
                self.advance_after_transition(parked, verdict, output);
            }
        }
    }

    /// Apply a pre-dispatch verdict.
    fn advance_after_pre_dispatch(
        &mut self,
        parked: ParkedDecision,
        verdict: GateVerdict,
        output: &mut TickOutput,
    ) {
        output.events.push(PipelineEvent::GateVerdictReceived {
            artifact_id: parked.decision.artifact_id.to_string(),
            gate_point: GatePoint::PreDispatch,
            verdict: summarize(&verdict),
        });

        match verdict {
            GateVerdict::Deny { reason } => {
                output.events.push(PipelineEvent::Error(format!(
                    "pre-dispatch gate denied for {}: {}",
                    parked.decision.artifact_id, reason
                )));
                // Park cleared — kernel may re-propose next tick.
            }
            GateVerdict::Escalate { .. } => {
                // forge invariant #5: park non-blockingly. We clear our
                // own park so the kernel can re-propose; the escalation
                // sits on the spine waiting for a delegate.
            }
            GateVerdict::Allow | GateVerdict::Modify { .. } => {
                self.kick_off_shaping(parked, output);
            }
        }
    }

    /// Pre-dispatch passed: emit the shaping request and re-park
    /// awaiting the result.
    fn kick_off_shaping(&mut self, parked: ParkedDecision, output: &mut TickOutput) {
        let request_id = ulid::Ulid::new().to_string();
        let shaping_request = ShapingRequest {
            request_id: request_id.clone(),
            artifact_id: parked.decision.artifact_id.clone(),
            target_version: parked.decision.target_version,
            shaping_intent: parked.decision.shaping_intent.clone(),
            inputs: parked.decision.inputs.clone(),
            constraints: parked.decision.constraints.clone(),
            deadline: parked.decision.deadline,
            // Legacy kernel path predates per-workflow ownership (issue
            // #156). Direct-shaping decisions don't flow through a
            // workflow, so there's no `workflow.created_by` to forward;
            // stiglab will spawn the agent without OAuth and the
            // resulting failure surfaces via session_failed.
            created_by: None,
        };

        output.events.push(PipelineEvent::ShapingDispatched {
            request_id: request_id.clone(),
            artifact_id: parked.decision.artifact_id.to_string(),
            target_version: parked.decision.target_version,
            request: shaping_request,
        });

        self.parked = Some(ParkedDecision {
            decision: parked.decision,
            artifact_snapshot: parked.artifact_snapshot,
            stage: ParkStage::AwaitingShapingResult { request_id },
        });
    }

    /// Apply a shaping result.
    fn advance_after_shaping(
        &mut self,
        parked: ParkedDecision,
        result: ShapingResult,
        output: &mut TickOutput,
    ) {
        let request_id = match &parked.stage {
            ParkStage::AwaitingShapingResult { request_id } => request_id.clone(),
            _ => unreachable!("advance_after_shaping called from non-shaping stage"),
        };

        output.events.push(PipelineEvent::ShapingReturned {
            request_id,
            artifact_id: parked.decision.artifact_id.to_string(),
            outcome: format!("{:?}", result.outcome),
        });

        // Short-circuit on unsuccessful outcomes — don't advance state
        // (forge-v0.1 §5.4: Failed/Aborted → artifact stays in previous
        // state). Park cleared so the kernel can re-propose.
        if matches!(
            result.outcome,
            ShapingOutcome::Failed | ShapingOutcome::Aborted
        ) {
            output.events.push(PipelineEvent::Error(format!(
                "shaping {:?} for {}: not advancing state",
                result.outcome, parked.decision.artifact_id
            )));
            return;
        }

        // Pre-dispatch passed and shaping succeeded; emit the
        // state-transition gate and re-park.
        let gate_id = ulid::Ulid::new().to_string();
        let gate_request =
            build_transition_gate(&parked.decision, &parked.artifact_snapshot, &result);
        output.events.push(PipelineEvent::GateRequested {
            gate_id: gate_id.clone(),
            artifact_id: parked.decision.artifact_id.to_string(),
            gate_point: GatePoint::StateTransition,
            request: gate_request,
        });
        self.parked = Some(ParkedDecision {
            decision: parked.decision,
            artifact_snapshot: parked.artifact_snapshot,
            stage: ParkStage::AwaitingTransitionVerdict {
                gate_id,
                result: Box::new(result),
            },
        });
    }

    /// Apply a state-transition verdict — the terminal stage of a
    /// successful pipeline run. Destructures `parked` so the boxed
    /// `ShapingResult` can move out by value (no clone of a payload
    /// the box was put there to keep small).
    fn advance_after_transition(
        &mut self,
        parked: ParkedDecision,
        verdict: GateVerdict,
        output: &mut TickOutput,
    ) {
        let ParkedDecision {
            decision,
            artifact_snapshot,
            stage,
        } = parked;
        let result = match stage {
            ParkStage::AwaitingTransitionVerdict { result, .. } => *result,
            _ => unreachable!("advance_after_transition called from non-transition stage"),
        };

        output.events.push(PipelineEvent::GateVerdictReceived {
            artifact_id: decision.artifact_id.to_string(),
            gate_point: GatePoint::StateTransition,
            verdict: summarize(&verdict),
        });

        match verdict {
            GateVerdict::Allow | GateVerdict::Modify { .. } => {
                self.apply_transition(decision, artifact_snapshot, result, output);
            }
            GateVerdict::Deny { reason } => {
                output.events.push(PipelineEvent::Error(format!(
                    "state transition gate denied for {}: {}",
                    decision.artifact_id, reason
                )));
            }
            GateVerdict::Escalate { .. } => {
                // forge invariant #5: clear park, let kernel re-propose.
            }
        }
    }

    /// Seal-if-Released and advance the artifact. Called only from
    /// the Allow/Modify branch of [`Self::advance_after_transition`],
    /// which destructures the parked decision so the boxed
    /// `ShapingResult` can flow through by value (no clone).
    fn apply_transition(
        &mut self,
        decision: ShapingDecision,
        artifact_snapshot: Artifact,
        result: ShapingResult,
        output: &mut TickOutput,
    ) {
        let from_state = artifact_snapshot.state;
        let target_state = decision.target_state;

        // warehouse-and-delivery-v0.1 §5.1: Released implies a sealed
        // bundle. Seal before advancing — if it fails the transition
        // aborts and the artifact stays in its prior state (kernel
        // re-proposes on a follow-up tick).
        let sealing_release =
            target_state == ArtifactState::Released && result.outcome == ShapingOutcome::Completed;
        let sealed = if sealing_release {
            match &self.warehouse {
                Some(warehouse) => match warehouse.seal_release(&decision.artifact_id, &result) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        output.events.push(PipelineEvent::Error(format!(
                            "warehouse seal failed for {}: {}",
                            decision.artifact_id, e
                        )));
                        return;
                    }
                },
                None => None,
            }
        } else {
            None
        };

        match self
            .store
            .advance(&decision.artifact_id, target_state, &result)
        {
            Ok(()) => {
                output.events.push(PipelineEvent::ArtifactAdvanced {
                    artifact_id: decision.artifact_id.to_string(),
                    from_state,
                    to_state: target_state,
                });

                if let Some(sealed) = sealed {
                    self.store
                        .record_version(&decision.artifact_id, sealed.bundle_id.clone());
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
}

/// Build the standard pre-dispatch [`GateRequest`] for a decision.
fn build_pre_dispatch_gate(decision: &ShapingDecision, artifact: &Artifact) -> GateRequest {
    GateRequest {
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
    }
}

/// Build the state-transition [`GateRequest`] sent after a successful
/// shaping. The `payload` carries the full `ShapingResult` so Synodic
/// can branch on the artifact's actual outputs.
fn build_transition_gate(
    decision: &ShapingDecision,
    artifact: &Artifact,
    result: &ShapingResult,
) -> GateRequest {
    GateRequest {
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
            payload: serde_json::to_value(result).expect("ShapingResult must be serializable"),
        },
    }
}

fn summarize(verdict: &GateVerdict) -> VerdictSummary {
    match verdict {
        GateVerdict::Allow => VerdictSummary::Allow,
        GateVerdict::Deny { .. } => VerdictSummary::Deny,
        GateVerdict::Modify { .. } => VerdictSummary::Modify,
        GateVerdict::Escalate { .. } => VerdictSummary::Escalate,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[path = "pipeline_tests.rs"]
#[cfg(test)]
mod tests;
