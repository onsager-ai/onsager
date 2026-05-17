//! [`HumanExecutor`] (EXE-06, issue #358) — pauses execution awaiting
//! an out-of-band approval decision, then emits a Deterministic-Human
//! attestation artifact.
//!
//! ## Shape
//!
//! Some workflows require a human approval step before proceeding. A
//! Human node parks the run, surfaces a prompt via the
//! `node.awaiting_human` spine event, and blocks until something
//! external (a dashboard action, a CLI command, a webhook) signals a
//! decision. On approval the node emits a `Deterministic { source:
//! Human }` attestation — a human has explicitly attested the artifact.
//! On rejection (or timeout) the node fails the run.
//!
//! ## Both halves of the [Executor trait pair](crate::executor)
//!
//! Like [`crate::VerifyExecutor`] and [`crate::AgentExecutor`], the
//! substrate-side trait and the runtime-side trait are both implemented
//! on the same `HumanExecutor` struct:
//!
//! - **substrate side** ([`onsager_substrate::executor::Executor`]):
//!   serializable via [`typetag`] under `kind = "human"`. Declared
//!   provenance is always `Deterministic { source: Human }` — by the
//!   time this node emits an artifact, a human has attested it.
//! - **runtime side** ([`crate::Executor`]): emits
//!   `node.awaiting_human`, awaits a decision via the configured
//!   [`ApprovalSource`], then emits either `node.human_approved` (and
//!   produces the attestation) or `node.human_rejected` (and returns
//!   [`ExecutorError::Failed`]).
//!
//! ## Provenance and ADR 0010
//!
//! Per [ADR 0010](../../../docs/adr/0010-provenance-as-substrate-first-class.md),
//! Verify is the only kernel-recognized upgrade path from `Uncertain`
//! to `Deterministic`. The static validator's invariant 2 (ADR 0018)
//! enforces this by special-casing `executor_kind() == "verify"`.
//!
//! Human declares `Deterministic { source: Human }` unconditionally, in
//! the same shape as Verify. The kernel does *not* exempt Human from
//! invariant 2 — workflows that route an `Uncertain` artifact directly
//! into a Human node fail validation. The mental model is that the
//! human's attestation is a fresh artifact (the act of approval), not a
//! transformation of inputs; runtime inputs are informational context
//! for the approver. If a workflow needs a Human attestation downstream
//! of an `Uncertain` producer, a Verify node must sit in between.
//!
//! ## The ApprovalSource port
//!
//! How the decision reaches the executor is a wiring concern, not part
//! of the workflow template's serialized form. The `source` field is
//! `#[serde(skip)]`; a deserialized `HumanExecutor` carries
//! [`UnconfiguredApprovalSource`] and will error if `execute` is called
//! before [`HumanExecutor::with_source`] rewires it. Production
//! deployments back the port with a spine-listener adapter that watches
//! for portal-emitted approval events; tests substitute
//! [`StubApprovalSource`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use onsager_artifact::{Artifact, Kind, NodeId, Provenance, SourceTag};
use onsager_substrate::events as se;
use onsager_substrate::executor::Executor as SubstrateExecutor;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor as RuntimeExecutor;
use crate::scheduler::PlanId;

/// Wire-format tag for the Human executor. Shared by the substrate
/// typetag discriminator and the runtime registry key.
pub const HUMAN_KIND: &str = "human";

// ---------------------------------------------------------------------------
// Decision + Source abstraction
// ---------------------------------------------------------------------------

/// The decision returned by an [`ApprovalSource`].
///
/// Carries the actor identity (and, on rejection, an optional reason)
/// so the corresponding `node.human_approved` / `node.human_rejected`
/// spine event matches the [`onsager_substrate::events`] payload shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// The human (or delegate) approved this node.
    Approved {
        /// Actor identifier — `"human:<id>"` for a dashboard user,
        /// `"supervisor"` for a delegate agent. Surfaces verbatim on
        /// `node.human_approved`.
        approved_by: String,
    },
    /// The human (or delegate) rejected this node.
    Rejected {
        /// Actor identifier — same shape as `approved_by` above.
        rejected_by: String,
        /// Free-text justification carried into the audit trail.
        reason: Option<String>,
    },
}

/// Error returned by an [`ApprovalSource::await_decision`] call.
#[derive(Debug, Error)]
#[error("approval source error: {0}")]
pub struct ApprovalSourceError(String);

impl ApprovalSourceError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Port over the actual approval-delivery backend.
///
/// Production wiring backs this with a spine-listener adapter that
/// watches `node.human_approved` / `node.human_rejected` events and
/// resolves the matching pending Human node. Tests substitute
/// [`StubApprovalSource`] (or a custom impl) so an end-to-end run can
/// drive the executor without a live spine.
///
/// Object-safe by design: held inside [`HumanExecutor`] as
/// `Arc<dyn ApprovalSource>`.
#[async_trait]
pub trait ApprovalSource: Send + Sync + std::fmt::Debug {
    /// Block until a decision arrives for this `(plan_id, node_id)`.
    /// The implementation is responsible for routing — multiple Human
    /// nodes may be pending at once.
    async fn await_decision(
        &self,
        plan_id: &PlanId,
        node_id: NodeId,
    ) -> Result<ApprovalDecision, ApprovalSourceError>;
}

/// Default `ApprovalSource` placeholder.
///
/// Carried by every freshly-deserialized `HumanExecutor` (the `source`
/// field is `#[serde(skip)]`). Calling `await_decision` always errors;
/// substrate-side validation does not touch the source, so this default
/// is invisible to the kernel invariant checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnconfiguredApprovalSource;

#[async_trait]
impl ApprovalSource for UnconfiguredApprovalSource {
    async fn await_decision(
        &self,
        _plan_id: &PlanId,
        _node_id: NodeId,
    ) -> Result<ApprovalDecision, ApprovalSourceError> {
        Err(ApprovalSourceError::new(
            "HumanExecutor has no configured approval source — call `with_source(..)` before registering",
        ))
    }
}

/// In-memory `ApprovalSource` for tests and early-bringup wiring.
/// Returns the configured decision immediately on every call, ignoring
/// the `(plan_id, node_id)` routing.
#[derive(Debug, Clone)]
pub struct StubApprovalSource {
    pub decision: ApprovalDecision,
}

impl StubApprovalSource {
    /// Build a stub that always approves on behalf of `approved_by`.
    pub fn approved(approved_by: impl Into<String>) -> Self {
        Self {
            decision: ApprovalDecision::Approved {
                approved_by: approved_by.into(),
            },
        }
    }

    /// Build a stub that always rejects on behalf of `rejected_by`.
    pub fn rejected(rejected_by: impl Into<String>, reason: Option<String>) -> Self {
        Self {
            decision: ApprovalDecision::Rejected {
                rejected_by: rejected_by.into(),
                reason,
            },
        }
    }
}

#[async_trait]
impl ApprovalSource for StubApprovalSource {
    async fn await_decision(
        &self,
        _plan_id: &PlanId,
        _node_id: NodeId,
    ) -> Result<ApprovalDecision, ApprovalSourceError> {
        Ok(self.decision.clone())
    }
}

// ---------------------------------------------------------------------------
// HumanExecutor
// ---------------------------------------------------------------------------

/// The Human executor.
///
/// Per-instance: each Human node in a workflow carries its own `prompt`
/// (shown to the approver) and optional `timeout_secs`. The `source`
/// field is runtime wiring — see the module-level § "The ApprovalSource
/// port".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanExecutor {
    /// Free-text prompt shown to the approver via the
    /// `node.awaiting_human` spine event. Rendered in the dashboard
    /// HITL inbox.
    pub prompt: String,
    /// If set, the executor fails the node (as a rejection) if no
    /// decision arrives within `timeout_secs` seconds. `None` waits
    /// indefinitely — the dashboard / operator owns the parked run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(skip, default = "default_source")]
    source: Arc<dyn ApprovalSource>,
}

fn default_source() -> Arc<dyn ApprovalSource> {
    Arc::new(UnconfiguredApprovalSource)
}

impl HumanExecutor {
    /// Build a Human executor with `prompt` and no timeout. The source
    /// defaults to [`UnconfiguredApprovalSource`]; layer the real
    /// source in with [`Self::with_source`] before registering.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            timeout_secs: None,
            source: default_source(),
        }
    }

    /// Set the approval-await timeout. Exceeded → the executor emits
    /// `node.human_rejected` (with `rejected_by = "timeout"`) and
    /// returns [`ExecutorError::Failed`].
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Replace the runtime approval source. Required before `execute`
    /// does anything useful — the default errors loudly.
    pub fn with_source(mut self, source: Arc<dyn ApprovalSource>) -> Self {
        self.source = source;
        self
    }

    fn attested_provenance() -> Provenance {
        Provenance::Deterministic {
            source: SourceTag::Human,
        }
    }
}

// ---------------------------------------------------------------------------
// Substrate (static) trait — typetag round-trip + kind discriminator
// ---------------------------------------------------------------------------

#[typetag::serde(name = "human")]
impl SubstrateExecutor for HumanExecutor {
    fn executor_kind(&self) -> &'static str {
        HUMAN_KIND
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        // Always Deterministic-Human — a human has attested the
        // artifact. The kernel does NOT exempt Human from invariant 2
        // (only Verify is exempted, per ADR 0010 § "Verify is the only
        // upgrade path"), so a workflow that routes Uncertain into a
        // Human node fails validation. The cure is a Verify node in
        // between, not a kernel change.
        Self::attested_provenance()
    }
}

// ---------------------------------------------------------------------------
// Runtime (async) trait — registry dispatch
// ---------------------------------------------------------------------------

#[async_trait]
impl RuntimeExecutor for HumanExecutor {
    fn executor_kind(&self) -> &'static str {
        HUMAN_KIND
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Self::attested_provenance()
    }

    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let plan_id_str = ctx.plan_id.as_str().to_string();

        // 1. Surface the pause to whoever is watching — dashboard
        //    HITL inbox, CLI tail, supervising agent. Emitted before
        //    we start awaiting so a fast-arriving decision can't race
        //    past the "awaiting" record on the timeline.
        emit_event(
            &ctx,
            se::KIND_NODE_AWAITING_HUMAN,
            &se::NodeAwaitingHuman {
                plan_id: plan_id_str.clone(),
                node_id: ctx.node_id,
                prompt: self.prompt.clone(),
            },
        )
        .await;

        // 2. Block awaiting the decision, optionally with a timeout.
        //    A timeout-as-rejection is the v1 nice-to-have flagged in
        //    the issue's Notes; it surfaces as
        //    `rejected_by = "timeout"` so the audit trail
        //    distinguishes it from an explicit deny.
        let decision = await_decision(self.source.as_ref(), &ctx, self.timeout_secs).await?;

        // 3. Branch on the decision. The corresponding spine event is
        //    emitted from here (not from the external trigger) so the
        //    audit trail has a single, authoritative record per node
        //    even if the trigger machinery is multi-step (portal
        //    handler → notifier → ApprovalSource).
        match decision {
            ApprovalDecision::Approved { approved_by } => {
                emit_event(
                    &ctx,
                    se::KIND_NODE_HUMAN_APPROVED,
                    &se::NodeHumanApproved {
                        plan_id: plan_id_str,
                        node_id: ctx.node_id,
                        approved_by: approved_by.clone(),
                    },
                )
                .await;
                Ok(ExecutorOutputs::single(
                    onsager_artifact::ArtifactId::generate(),
                    build_attestation(ctx.node_id, &approved_by, &self.prompt),
                ))
            }
            ApprovalDecision::Rejected {
                rejected_by,
                reason,
            } => {
                emit_event(
                    &ctx,
                    se::KIND_NODE_HUMAN_REJECTED,
                    &se::NodeHumanRejected {
                        plan_id: plan_id_str,
                        node_id: ctx.node_id,
                        rejected_by: rejected_by.clone(),
                        reason: reason.clone(),
                    },
                )
                .await;
                let detail = reason.as_deref().unwrap_or("(no reason given)");
                Err(ExecutorError::Failed(format!(
                    "human rejected by {rejected_by}: {detail}"
                )))
            }
        }
    }
}

/// Await a decision through `source`, applying `timeout_secs` if set.
/// On timeout, emit `node.human_rejected { rejected_by: "timeout" }`
/// and return `Err`. A non-timeout source error propagates as
/// `ExecutorError::Failed` — the executor itself does not emit a
/// rejection event in that case (the source is down, not the human).
async fn await_decision(
    source: &dyn ApprovalSource,
    ctx: &ExecutorContext,
    timeout_secs: Option<u64>,
) -> Result<ApprovalDecision, ExecutorError> {
    let fut = source.await_decision(&ctx.plan_id, ctx.node_id);
    let result = match timeout_secs {
        Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), fut).await {
            Ok(r) => r,
            Err(_) => {
                let plan_id_str = ctx.plan_id.as_str().to_string();
                emit_event(
                    ctx,
                    se::KIND_NODE_HUMAN_REJECTED,
                    &se::NodeHumanRejected {
                        plan_id: plan_id_str,
                        node_id: ctx.node_id,
                        rejected_by: "timeout".to_string(),
                        reason: Some(format!("no decision within {secs}s")),
                    },
                )
                .await;
                return Err(ExecutorError::Failed(format!(
                    "human approval timed out after {secs}s"
                )));
            }
        },
        None => fut.await,
    };
    result.map_err(|e| ExecutorError::Failed(e.to_string()))
}

/// Best-effort spine emit for a Human lifecycle event. A failed emit
/// logs a warning and is not propagated — a missed lifecycle event
/// must not stall the actual execution (the decision still resolves;
/// the dashboard timeline loses one row).
async fn emit_event<T: serde::Serialize>(ctx: &ExecutorContext, kind: &str, payload: &T) {
    let payload = serde_json::to_value(payload).expect("substrate event payload must serialize");
    if let Err(e) = ctx.spine.emit(kind, payload).await {
        tracing::warn!(
            plan = %ctx.plan_id,
            node = %ctx.node_id,
            kind,
            "human executor spine emit failed: {e}",
        );
    }
}

fn build_attestation(node_id: NodeId, approved_by: &str, prompt: &str) -> Artifact {
    let mut art = Artifact::new(
        Kind::Document,
        format!("human.approved by {approved_by}: {prompt}"),
        "kernel",
        "human",
        vec![],
    );
    art.provenance = HumanExecutor::attested_provenance();
    art.produced_by_node = Some(node_id);
    art
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch;
    use crate::registry::ExecutorRegistry;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::ArtifactId;
    use std::sync::Mutex;

    fn ctx_for(node_id: NodeId) -> (ExecutorContext, Arc<MockSpine>) {
        let spine = Arc::new(MockSpine::default());
        let ctx = ExecutorContext {
            plan_id: PlanId::generate(),
            node_id,
            inputs: vec![],
            spine: Arc::clone(&spine) as Arc<dyn crate::SpineClient>,
            subworkflow_ref: None,
        };
        (ctx, spine)
    }

    // -----------------------------------------------------------------
    // Provenance + kind: Human always declares Deterministic-Human and
    // reports `executor_kind() == "human"` on both traits.
    // -----------------------------------------------------------------

    #[test]
    fn executor_kind_is_human_on_both_traits() {
        let h = HumanExecutor::new("approve?");
        assert_eq!(SubstrateExecutor::executor_kind(&h), "human");
        assert_eq!(RuntimeExecutor::executor_kind(&h), "human");
        assert_eq!(HUMAN_KIND, "human");
    }

    #[test]
    fn declared_provenance_is_deterministic_human_regardless_of_inputs() {
        let h = HumanExecutor::new("approve?");
        let expected = Provenance::Deterministic {
            source: SourceTag::Human,
        };
        // No inputs.
        assert_eq!(SubstrateExecutor::declared_provenance(&h, &[]), expected);
        assert_eq!(RuntimeExecutor::declared_provenance(&h, &[]), expected);
        // Even with Uncertain inputs the *declared* shape stays
        // Deterministic-Human — the kernel's invariant 2 (which only
        // exempts Verify) is what catches Uncertain → Human misuse at
        // validation time.
        let with_uncertain = [Provenance::Uncertain {
            source: SourceTag::Agent,
        }];
        assert_eq!(
            SubstrateExecutor::declared_provenance(&h, &with_uncertain),
            expected,
        );
        assert_eq!(
            RuntimeExecutor::declared_provenance(&h, &with_uncertain),
            expected,
        );
    }

    // -----------------------------------------------------------------
    // Execute: approval / rejection / timeout / source failure.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_emits_awaiting_then_approved_and_produces_attestation() {
        // The acceptance test from the issue: workflow pauses
        // (`node.awaiting_human` emitted), an external trigger
        // approves, the workflow resumes with Deterministic-Human
        // provenance.
        let h = HumanExecutor::new("approve the change?")
            .with_source(Arc::new(StubApprovalSource::approved("human:42")));
        let node_id = NodeId::generate();
        let (ctx, spine) = ctx_for(node_id);
        let plan_id = ctx.plan_id.clone();

        let outputs = h.execute(ctx).await.expect("approved run succeeds");

        // One attestation artifact, Deterministic-Human, tagged with
        // the producing node.
        assert_eq!(outputs.artifacts.len(), 1);
        let (_id, art) = &outputs.artifacts[0];
        assert_eq!(
            art.provenance,
            Provenance::Deterministic {
                source: SourceTag::Human,
            }
        );
        assert_eq!(art.produced_by_node, Some(node_id));
        assert!(art.name.starts_with("human.approved"), "{}", art.name);
        assert!(art.name.contains("human:42"), "{}", art.name);

        // Spine emits, in order: awaiting then approved. (Strict
        // ordering matters — a fast approver mustn't race past the
        // pause record on the dashboard timeline.)
        let emitted = spine.emitted.lock().unwrap();
        let kinds: Vec<_> = emitted.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            vec![se::KIND_NODE_AWAITING_HUMAN, se::KIND_NODE_HUMAN_APPROVED],
        );

        // Awaiting payload carries the prompt.
        let (_, awaiting) = emitted
            .iter()
            .find(|(k, _)| k == se::KIND_NODE_AWAITING_HUMAN)
            .unwrap();
        assert_eq!(awaiting["plan_id"], plan_id.as_str());
        assert_eq!(awaiting["prompt"], "approve the change?");

        // Approved payload carries the actor.
        let (_, approved) = emitted
            .iter()
            .find(|(k, _)| k == se::KIND_NODE_HUMAN_APPROVED)
            .unwrap();
        assert_eq!(approved["approved_by"], "human:42");
    }

    #[tokio::test]
    async fn execute_emits_awaiting_then_rejected_and_returns_failed() {
        let h = HumanExecutor::new("approve?").with_source(Arc::new(StubApprovalSource::rejected(
            "human:7",
            Some("scope creep".to_string()),
        )));
        let (ctx, spine) = ctx_for(NodeId::generate());

        let err = h.execute(ctx).await.expect_err("rejection fails the node");
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("rejected"), "{msg}");
                assert!(msg.contains("human:7"), "{msg}");
                assert!(msg.contains("scope creep"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        let emitted = spine.emitted.lock().unwrap();
        let kinds: Vec<_> = emitted.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            vec![se::KIND_NODE_AWAITING_HUMAN, se::KIND_NODE_HUMAN_REJECTED],
        );

        let (_, rejected) = emitted
            .iter()
            .find(|(k, _)| k == se::KIND_NODE_HUMAN_REJECTED)
            .unwrap();
        assert_eq!(rejected["rejected_by"], "human:7");
        assert_eq!(rejected["reason"], "scope creep");
    }

    #[tokio::test]
    async fn execute_without_source_errors_clearly() {
        // Default `UnconfiguredApprovalSource` is what a freshly-
        // deserialized HumanExecutor carries. Calling execute on it
        // surfaces the wiring miss as a Failed error, not a panic.
        let h = HumanExecutor::new("approve?");
        let (ctx, spine) = ctx_for(NodeId::generate());
        let err = h.execute(ctx).await.expect_err("no source is an error");
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("with_source"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        // Awaiting fired first (the prompt is surfaced to the
        // dashboard regardless), but no decision event was emitted —
        // the source itself failed before resolving.
        let emitted = spine.emitted.lock().unwrap();
        let kinds: Vec<_> = emitted.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kinds, vec![se::KIND_NODE_AWAITING_HUMAN]);
    }

    #[tokio::test]
    async fn execute_with_timeout_emits_rejection_when_decision_never_arrives() {
        // A source that blocks forever — proves the timeout fires.
        #[derive(Debug)]
        struct NeverDecides;
        #[async_trait]
        impl ApprovalSource for NeverDecides {
            async fn await_decision(
                &self,
                _: &PlanId,
                _: NodeId,
            ) -> Result<ApprovalDecision, ApprovalSourceError> {
                // Use a long sleep that the timeout pre-empts. A bare
                // `pending::<_>()` would also work but this shape is
                // closer to what a real listener does.
                tokio::time::sleep(Duration::from_secs(3600)).await;
                unreachable!("timeout must fire first")
            }
        }

        // Use tokio's paused-time machinery so the test doesn't
        // actually wait a real second.
        tokio::time::pause();
        let h = HumanExecutor::new("approve?")
            .with_timeout_secs(1)
            .with_source(Arc::new(NeverDecides));
        let (ctx, spine) = ctx_for(NodeId::generate());

        let exec_handle = tokio::spawn(async move { h.execute(ctx).await });
        // Advance virtual time past the timeout. `auto_advance` would
        // also work; this is explicit about the boundary.
        tokio::time::advance(Duration::from_secs(2)).await;
        let err = exec_handle
            .await
            .expect("task joins")
            .expect_err("timeout fails the node");

        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("timed out"), "{msg}");
                assert!(msg.contains("1s"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        let emitted = spine.emitted.lock().unwrap();
        let kinds: Vec<_> = emitted.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            vec![se::KIND_NODE_AWAITING_HUMAN, se::KIND_NODE_HUMAN_REJECTED],
        );
        let (_, rejected) = emitted
            .iter()
            .find(|(k, _)| k == se::KIND_NODE_HUMAN_REJECTED)
            .unwrap();
        assert_eq!(rejected["rejected_by"], "timeout");
        assert!(rejected["reason"].as_str().unwrap().contains("1s"));
    }

    // -----------------------------------------------------------------
    // ApprovalSource: routing is provided to the implementation.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn approval_source_receives_plan_and_node_for_routing() {
        // Verification that a real source — which would route by
        // (plan_id, node_id) — sees the same identifiers the executor
        // is running under. Captures the args so the test asserts on
        // them.
        #[derive(Debug, Default)]
        struct CapturingSource {
            seen: Mutex<Option<(PlanId, NodeId)>>,
        }
        #[async_trait]
        impl ApprovalSource for CapturingSource {
            async fn await_decision(
                &self,
                plan_id: &PlanId,
                node_id: NodeId,
            ) -> Result<ApprovalDecision, ApprovalSourceError> {
                *self.seen.lock().unwrap() = Some((plan_id.clone(), node_id));
                Ok(ApprovalDecision::Approved {
                    approved_by: "human:99".into(),
                })
            }
        }
        let source: Arc<CapturingSource> = Arc::new(CapturingSource::default());
        let h = HumanExecutor::new("approve?").with_source(source.clone());
        let node_id = NodeId::generate();
        let (ctx, _spine) = ctx_for(node_id);
        let plan_id = ctx.plan_id.clone();
        h.execute(ctx).await.unwrap();
        let seen = source.seen.lock().unwrap().clone().unwrap();
        assert_eq!(seen.0.as_str(), plan_id.as_str());
        assert_eq!(seen.1, node_id);
    }

    // -----------------------------------------------------------------
    // Serde round-trip via typetag — substrate trait object
    // serializes to `{"kind": "human", ...}` and back.
    // -----------------------------------------------------------------

    #[test]
    fn human_executor_roundtrips_as_substrate_trait_object() {
        let original: Box<dyn SubstrateExecutor> =
            Box::new(HumanExecutor::new("approve?").with_timeout_secs(60));
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json["kind"], "human");
        assert_eq!(json["prompt"], "approve?");
        assert_eq!(json["timeout_secs"], 60);

        let roundtrip: Box<dyn SubstrateExecutor> = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.executor_kind(), "human");
        assert_eq!(
            roundtrip.declared_provenance(&[]),
            Provenance::Deterministic {
                source: SourceTag::Human,
            },
        );
    }

    #[test]
    fn human_executor_omits_none_timeout_on_wire() {
        // No timeout → `timeout_secs` field omitted (schema-stability
        // promise from `onsager_substrate::events` — additive `Option`
        // fields with `skip_serializing_if = "Option::is_none"`).
        let original: Box<dyn SubstrateExecutor> = Box::new(HumanExecutor::new("approve?"));
        let json = serde_json::to_value(&original).unwrap();
        assert!(
            json.get("timeout_secs").is_none(),
            "None timeout must be omitted, got: {json}"
        );
    }

    // -----------------------------------------------------------------
    // Dispatch through registry — Human resolves through the runtime
    // registry exactly like any other executor.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn dispatch_through_registry_runs_the_human_executor() {
        use onsager_substrate::workflow::Node;

        let registered = HumanExecutor::new("approve?")
            .with_source(Arc::new(StubApprovalSource::approved("human:42")));
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(registered));

        let node = Node {
            id: NodeId::generate(),
            executor: Box::new(HumanExecutor::new("approve?")),
            inputs: vec![],
            outputs: vec![],
        };
        let (ctx, _spine) = ctx_for(node.id);
        let outputs = dispatch(&registry, &node, ctx).await.unwrap();
        assert_eq!(outputs.artifacts.len(), 1);
        let (_, art) = &outputs.artifacts[0];
        assert_eq!(
            art.provenance,
            Provenance::Deterministic {
                source: SourceTag::Human,
            },
        );
    }

    // -----------------------------------------------------------------
    // Compile-time check: object-safe on the runtime trait.
    // -----------------------------------------------------------------

    #[test]
    fn human_executor_trait_object_safe() {
        let _boxed: Box<dyn RuntimeExecutor> = Box::new(HumanExecutor::new("approve?"));
        let _arced: Arc<dyn RuntimeExecutor> = Arc::new(HumanExecutor::new("approve?"));
    }

    // -----------------------------------------------------------------
    // Static validation: a Human node downstream of a Deterministic
    // input clears invariant 2 cleanly; a Human node downstream of an
    // Uncertain input fails it (Human is NOT a kernel-recognized
    // upgrade path — only Verify is).
    // -----------------------------------------------------------------

    #[test]
    fn workflow_with_human_after_deterministic_input_validates() {
        use onsager_substrate::ids::EdgeId;
        use onsager_substrate::validate::validate_workflow;
        use onsager_substrate::workflow::{Edge, EdgeRef, Node, Workflow};

        // External-deterministic entry into Human. Human declares
        // Deterministic-Human; no Uncertain anywhere, invariant 2
        // doesn't fire.
        let entry_edge = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_entry"),
            requires_deterministic: false,
        };
        let human_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_human"),
            requires_deterministic: false,
        };
        let w = Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(HumanExecutor::new("approve?")),
                inputs: vec![EdgeRef::new(entry_edge.id)],
                outputs: vec![EdgeRef::new(human_out.id)],
            }],
            edges: vec![entry_edge, human_out],
            entry_specs: vec![],
            output_specs: vec![],
        };
        validate_workflow(&w, &()).expect("Human on deterministic input must validate");
    }

    #[test]
    fn workflow_with_human_after_uncertain_input_fails_invariant_2() {
        use onsager_substrate::ids::EdgeId;
        use onsager_substrate::validate::validate_workflow;
        use onsager_substrate::workflow::{Edge, EdgeRef, Node, Workflow};

        // Uncertain-emitting stub (stand-in for Agent — the real
        // executor lives in `agent.rs`). The kernel only checks
        // `executor_kind`, so any non-"verify" tag suffices upstream.
        #[derive(Debug, Default, Serialize, Deserialize)]
        struct AgentStub;
        #[typetag::serde(name = "test-human-agent-stub")]
        impl SubstrateExecutor for AgentStub {
            fn executor_kind(&self) -> &'static str {
                "test-human-agent-stub"
            }
            fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
                Provenance::Uncertain {
                    source: SourceTag::Agent,
                }
            }
        }

        let agent_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_agent"),
            requires_deterministic: false,
        };
        let human_out = Edge {
            id: EdgeId::generate(),
            artifact_id: ArtifactId::new("art_human"),
            requires_deterministic: false,
        };
        let w = Workflow {
            nodes: vec![
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(AgentStub),
                    inputs: vec![],
                    outputs: vec![EdgeRef::new(agent_out.id)],
                },
                Node {
                    id: NodeId::generate(),
                    executor: Box::new(HumanExecutor::new("approve?")),
                    inputs: vec![EdgeRef::new(agent_out.id)],
                    outputs: vec![EdgeRef::new(human_out.id)],
                },
            ],
            edges: vec![agent_out, human_out],
            entry_specs: vec![],
            output_specs: vec![],
        };
        let err = validate_workflow(&w, &()).unwrap_err();
        assert!(
            err.iter().any(|v| v.invariant == 2),
            "expected invariant 2 violation, got {err:?}",
        );
    }
}
