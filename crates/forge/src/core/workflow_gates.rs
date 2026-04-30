//! Live gate evaluator implementations (issue #80; spine-event flow per
//! spec #131 / ADR 0004 Lever C, phase 5 — see #148).
//!
//! Wires the four gate kinds to their production-time backends:
//!
//! - `agent-session` — emits `forge.shaping_dispatched` on first
//!   observation; the stiglab listener spawns the session and the
//!   `stiglab.session_completed` event lands a signal in the
//!   [`SignalCache`] that the next tick observes.
//! - `external-check` — consumes the cache, where the GitHub CI-event
//!   listener has written a `ci:<check_name>` signal.
//! - `governance` — emits `forge.gate_requested` on first observation
//!   and parks; the synodic listener evaluates the rule set and emits
//!   `synodic.gate_verdict`, which the verdict listener parks in
//!   [`PendingVerdicts`] for a later tick to claim.
//! - `manual-approval` — consumes the cache for the
//!   `signal_kind` declared on the gate (e.g. `pr_merged`,
//!   `dashboard_approve`).
//!
//! No HTTP roundtrips to sibling subsystems live in this module after
//! phase 5 — the seam between forge and stiglab/synodic is the spine
//! exclusively.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use onsager_artifact::Artifact;
use onsager_spine::factory_event::{FactoryEventKind, GatePoint};
use onsager_spine::protocol::{
    GateContext, GateRequest, GateVerdict, ProposedAction, ShapingRequest,
};
use onsager_spine::{EventMetadata, EventStore};

use super::pending::PendingVerdicts;
use super::signal_cache::{SignalCache, SignalOutcome};
use super::workflow::{GateOutcome, GateSpec, Workflow};

/// Signal kind used to mark the completion of an agent-session gate.
/// The session listener writes `SignalOutcome::Success` under this kind
/// on `stiglab.session_completed` for the matching artifact.
///
/// The stage runner clears this key on every stage advance so a prior
/// session's signal can't satisfy a later `agent-session` gate for the
/// same artifact (issue #80 copilot-review).
pub const AGENT_SESSION_SIGNAL: &str = "agent_session";

/// Signal-kind prefix for external CI checks. Full kind is
/// `ci:<check_name>` — e.g. `ci:ci/test`.
pub fn external_check_signal_kind(check_name: &str) -> String {
    format!("ci:{check_name}")
}

/// Default cap on agent-session dispatches per stage-runner pass. Keeps
/// a burst of new workflow artifacts from synchronously hammering
/// the spine under the Forge write lock (issue #80 copilot-review;
/// the tighter constraint pre-phase-5 was the Stiglab HTTP path, but
/// the same write-lock pressure applies to spine emits in this loop).
pub const DEFAULT_DISPATCH_BUDGET_PER_TICK: u32 = 4;

/// Emit a request event onto the spine. Production wires
/// [`SpineGateEmitter`] (wraps `EventStore`); tests use a capture impl
/// to assert the gate evaluator emitted what the spine listener
/// expects.
pub trait GateEmitter: Send + Sync {
    /// Emit `forge.shaping_dispatched` carrying the full
    /// [`ShapingRequest`] payload. Returns `true` when the spine
    /// accepted the append; `false` lets the caller defer marking the
    /// gate as dispatched so a transient append failure doesn't strand
    /// the artifact (see `evaluate_agent_session`).
    fn emit_shaping_dispatched(&self, request: &ShapingRequest) -> bool;

    /// Emit `forge.gate_requested` keyed on `gate_id`. Same return
    /// semantics as [`Self::emit_shaping_dispatched`].
    fn emit_gate_requested(&self, gate_id: &str, request: &GateRequest) -> bool;
}

/// Production [`GateEmitter`] backed by an [`EventStore`]. Calls into
/// `append_ext` from a synchronous context via
/// `block_in_place + block_on` — same pattern the legacy HTTP
/// dispatchers used. Errors are logged and surfaced to the caller as
/// `false` so it can decide whether to retry.
pub struct SpineGateEmitter {
    store: EventStore,
}

impl SpineGateEmitter {
    pub fn new(store: EventStore) -> Self {
        Self { store }
    }

    fn emit(&self, stream_id: &str, kind: FactoryEventKind) -> bool {
        let metadata = EventMetadata {
            actor: "forge".into(),
            ..Default::default()
        };
        let event_type = kind.event_type();
        let data = serde_json::to_value(&kind).expect("FactoryEventKind must serialize");
        let outcome = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.store
                    .append_ext(stream_id, "forge", event_type, data, &metadata, None),
            )
        });
        match outcome {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(
                    event_type,
                    stream_id,
                    "forge: spine emit failed for workflow gate: {e}"
                );
                false
            }
        }
    }
}

impl GateEmitter for SpineGateEmitter {
    fn emit_shaping_dispatched(&self, request: &ShapingRequest) -> bool {
        self.emit(
            &format!("forge:{}", request.artifact_id),
            FactoryEventKind::ForgeShapingDispatched {
                request_id: request.request_id.clone(),
                artifact_id: request.artifact_id.clone(),
                target_version: request.target_version,
                request: Some(request.clone()),
            },
        )
    }

    fn emit_gate_requested(&self, gate_id: &str, request: &GateRequest) -> bool {
        self.emit(
            &format!("forge:{}", request.context.artifact_id),
            FactoryEventKind::ForgeGateRequested {
                gate_id: gate_id.to_string(),
                artifact_id: request.context.artifact_id.clone(),
                gate_point: request.context.gate_point,
                request: Some(request.clone()),
            },
        )
    }
}

/// Live gate evaluator that backs the stage runner in production.
///
/// Phase 5 dropped the `<S: StiglabDispatcher, G: SynodicGate>`
/// generics: the evaluator now emits spine events through a
/// [`GateEmitter`] and claims governance verdicts from
/// [`PendingVerdicts`] (the shared map the verdict listener
/// populates). The signal cache continues to back the
/// `agent-session` / `external-check` / `manual-approval` gates.
pub struct LiveGateEvaluator<E: GateEmitter> {
    signals: SignalCache,
    /// Spine emitter for the gate-emit half of every dispatch.
    emitter: E,
    /// Verdicts the spine `gate_verdict_listener` parks for the
    /// pipeline + the workflow stage runner to claim. The pipeline and
    /// the stage runner share the same map without conflict — gate ids
    /// are ULID-unique per emit, so the keyspaces don't collide.
    pending_verdicts: PendingVerdicts,
    /// (artifact_id, stage_index) → request_id of the
    /// `forge.shaping_dispatched` event we emitted. Tracks "this gate
    /// has already kicked off a session" so we don't redispatch on
    /// every tick while waiting for the matching signal.
    dispatched: Mutex<HashMap<(String, u32), String>>,
    /// (artifact_id, stage_index) → gate_id of the
    /// `forge.gate_requested` event we emitted for a governance gate.
    /// Lets the next tick claim the matching `synodic.gate_verdict`
    /// from `pending_verdicts`.
    governance_in_flight: Mutex<HashMap<(String, u32), String>>,
    /// Remaining gate emissions allowed this tick. Refilled by
    /// [`reset_dispatch_budget`] once per stage-runner pass so a burst
    /// of new workflow artifacts can't synchronously hammer the spine
    /// under the Forge write lock.
    dispatch_budget: AtomicU32,
    /// Budget ceiling refilled by [`reset_dispatch_budget`].
    dispatch_budget_per_tick: u32,
}

impl<E: GateEmitter> LiveGateEvaluator<E> {
    pub fn new(signals: SignalCache, emitter: E, pending_verdicts: PendingVerdicts) -> Self {
        Self::with_budget(
            signals,
            emitter,
            pending_verdicts,
            DEFAULT_DISPATCH_BUDGET_PER_TICK,
        )
    }

    pub fn with_budget(
        signals: SignalCache,
        emitter: E,
        pending_verdicts: PendingVerdicts,
        dispatch_budget_per_tick: u32,
    ) -> Self {
        Self {
            signals,
            emitter,
            pending_verdicts,
            dispatched: Mutex::new(HashMap::new()),
            governance_in_flight: Mutex::new(HashMap::new()),
            dispatch_budget: AtomicU32::new(dispatch_budget_per_tick),
            dispatch_budget_per_tick,
        }
    }

    /// Refill the per-tick dispatch budget. Call once at the top of each
    /// stage-runner pass.
    pub fn reset_dispatch_budget(&self) {
        self.dispatch_budget
            .store(self.dispatch_budget_per_tick, Ordering::SeqCst);
    }

    fn try_consume_dispatch(&self) -> bool {
        // Relaxed `fetch_update`: only one tick task runs the runner at
        // a time (under the Forge write lock), so no CAS races.
        let mut current = self.dispatch_budget.load(Ordering::SeqCst);
        while current > 0 {
            match self.dispatch_budget.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
        false
    }

    fn already_dispatched(&self, artifact_id: &str, stage_index: u32) -> bool {
        let map = self.dispatched.lock().expect("dispatched map poisoned");
        map.contains_key(&(artifact_id.to_string(), stage_index))
    }

    fn mark_dispatched(&self, artifact_id: &str, stage_index: u32, request_id: &str) {
        let mut map = self.dispatched.lock().expect("dispatched map poisoned");
        map.insert(
            (artifact_id.to_string(), stage_index),
            request_id.to_string(),
        );
    }

    fn evaluate_agent_session(
        &self,
        artifact: &Artifact,
        stage_index: u32,
        shaping_intent: &serde_json::Value,
        created_by: Option<&str>,
    ) -> GateOutcome {
        // Happy path: signal has already arrived.
        if let Some(outcome) = self
            .signals
            .get(artifact.artifact_id.as_str(), AGENT_SESSION_SIGNAL)
        {
            return match outcome {
                SignalOutcome::Success => GateOutcome::Pass,
                SignalOutcome::Failure(reason) => GateOutcome::Fail(reason),
            };
        }

        // First observation: emit a shaping-dispatched event onto the
        // spine. The stiglab listener (added in phase 3) consumes it
        // and spawns the session; the matching
        // `stiglab.session_completed` event writes into the signal
        // cache and a later tick resolves the gate.
        //
        // Budget-gated so a burst of new workflow artifacts can't fire
        // N synchronous spine emits under the Forge write lock. If the
        // budget is exhausted this tick, return Pending and retry next
        // tick — the artifact stays at this stage.
        if !self.already_dispatched(artifact.artifact_id.as_str(), stage_index) {
            if !self.try_consume_dispatch() {
                tracing::debug!(
                    artifact_id = %artifact.artifact_id,
                    stage_index,
                    "workflow gate: agent-session dispatch budget exhausted this tick"
                );
                return GateOutcome::Pending;
            }
            let request = ShapingRequest {
                request_id: ulid::Ulid::new().to_string(),
                artifact_id: artifact.artifact_id.clone(),
                target_version: artifact.current_version + 1,
                shaping_intent: shaping_intent.clone(),
                inputs: vec![],
                constraints: vec![],
                deadline: None,
                // Owner identity (issue #156). Stiglab uses this to
                // decrypt the matching CLAUDE_CODE_OAUTH_TOKEN; without
                // it the agent boots with no auth and exits immediately.
                created_by: created_by.map(str::to_owned),
            };
            // Only mark the gate as dispatched when the spine
            // accepted the append. Marking unconditionally would
            // strand the artifact at this stage on transient
            // postgres/network errors — `already_dispatched` would
            // suppress retries forever, and the completion signal
            // can't arrive for a session that was never created.
            if self.emitter.emit_shaping_dispatched(&request) {
                self.mark_dispatched(
                    artifact.artifact_id.as_str(),
                    stage_index,
                    &request.request_id,
                );
            }
        }
        GateOutcome::Pending
    }

    fn evaluate_external_check(&self, artifact: &Artifact, check_name: &str) -> GateOutcome {
        let kind = external_check_signal_kind(check_name);
        match self.signals.get(artifact.artifact_id.as_str(), &kind) {
            Some(SignalOutcome::Success) => GateOutcome::Pass,
            Some(SignalOutcome::Failure(reason)) => GateOutcome::Fail(reason),
            None => GateOutcome::Pending,
        }
    }

    fn evaluate_governance(
        &self,
        artifact: &Artifact,
        stage_index: u32,
        gate_point: &Option<String>,
    ) -> GateOutcome {
        // Resume path: if we already emitted a `forge.gate_requested`
        // for this (artifact, stage_index), check the parking map for
        // the matching verdict.
        let in_flight_key = (artifact.artifact_id.as_str().to_string(), stage_index);
        let parked_gate_id = {
            let map = self
                .governance_in_flight
                .lock()
                .expect("governance_in_flight poisoned");
            map.get(&in_flight_key).cloned()
        };
        if let Some(gate_id) = parked_gate_id.as_deref() {
            if let Some(verdict) = self.pending_verdicts.take(gate_id) {
                // Verdict landed — clear our in-flight marker and map
                // it to a stage-runner outcome.
                self.governance_in_flight
                    .lock()
                    .expect("governance_in_flight poisoned")
                    .remove(&in_flight_key);
                return match verdict {
                    GateVerdict::Allow | GateVerdict::Modify { .. } => GateOutcome::Pass,
                    GateVerdict::Deny { reason } => GateOutcome::Fail(reason),
                    // Escalate: forge invariant #5 — park non-blockingly.
                    // The escalation sits on the spine for a delegate;
                    // the workflow stays at this stage until a follow-up
                    // tick re-emits and a new verdict arrives.
                    GateVerdict::Escalate { .. } => GateOutcome::Pending,
                };
            }
            // Verdict hasn't landed yet — keep waiting.
            return GateOutcome::Pending;
        }

        // First observation: emit the gate request, park the gate_id
        // for the next tick to claim against `pending_verdicts`.
        if !self.try_consume_dispatch() {
            tracing::debug!(
                artifact_id = %artifact.artifact_id,
                stage_index,
                "workflow gate: governance dispatch budget exhausted this tick"
            );
            return GateOutcome::Pending;
        }
        let point = gate_point
            .as_deref()
            .map(parse_gate_point)
            .unwrap_or(GatePoint::StateTransition);
        let request = GateRequest {
            context: GateContext {
                gate_point: point,
                artifact_id: artifact.artifact_id.clone(),
                artifact_kind: artifact.kind.clone(),
                current_state: artifact.state,
                target_state: None,
                extra: None,
            },
            proposed_action: ProposedAction {
                description: format!("workflow governance gate for {}", artifact.artifact_id),
                payload: serde_json::Value::Null,
            },
        };
        let gate_id = ulid::Ulid::new().to_string();
        if self.emitter.emit_gate_requested(&gate_id, &request) {
            self.governance_in_flight
                .lock()
                .expect("governance_in_flight poisoned")
                .insert(in_flight_key, gate_id);
        }
        GateOutcome::Pending
    }

    fn evaluate_manual_approval(&self, artifact: &Artifact, signal_kind: &str) -> GateOutcome {
        match self.signals.get(artifact.artifact_id.as_str(), signal_kind) {
            Some(SignalOutcome::Success) => GateOutcome::Pass,
            Some(SignalOutcome::Failure(reason)) => GateOutcome::Fail(reason),
            None => GateOutcome::Pending,
        }
    }
}

impl<E: GateEmitter> super::stage_runner::GateEvaluator for LiveGateEvaluator<E> {
    fn evaluate(
        &self,
        artifact: &Artifact,
        workflow: &Workflow,
        stage_index: u32,
        gate: &GateSpec,
    ) -> GateOutcome {
        match gate {
            GateSpec::AgentSession { shaping_intent } => self.evaluate_agent_session(
                artifact,
                stage_index,
                shaping_intent,
                workflow.created_by.as_deref(),
            ),
            GateSpec::ExternalCheck { check_name } => {
                self.evaluate_external_check(artifact, check_name)
            }
            GateSpec::Governance { gate_point } => {
                self.evaluate_governance(artifact, stage_index, gate_point)
            }
            GateSpec::ManualApproval { signal_kind } => {
                self.evaluate_manual_approval(artifact, signal_kind)
            }
        }
    }

    fn on_stage_advanced(&self, artifact_id: &onsager_artifact::ArtifactId, stage_index: u32) {
        // Clear the agent-session signal so a completed session can't
        // satisfy a later stage's `agent-session` gate for the same
        // artifact. Also drop the "already dispatched" marker and any
        // in-flight governance gate_id for this stage so a revise
        // cycle (if any) redispatches.
        self.signals
            .clear(artifact_id.as_str(), AGENT_SESSION_SIGNAL);
        let key = (artifact_id.as_str().to_string(), stage_index);
        self.dispatched
            .lock()
            .expect("dispatched map poisoned")
            .remove(&key);
        self.governance_in_flight
            .lock()
            .expect("governance_in_flight poisoned")
            .remove(&key);
    }
}

fn parse_gate_point(s: &str) -> GatePoint {
    match s {
        "pre_dispatch" => GatePoint::PreDispatch,
        "consumer_routing" => GatePoint::ConsumerRouting,
        "tool_level" => GatePoint::ToolLevel,
        _ => GatePoint::StateTransition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::stage_runner::GateEvaluator;
    use onsager_artifact::Kind;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Test [`GateEmitter`] that captures emitted events instead of
    /// touching a live spine. Lets unit tests assert "the gate emitted
    /// the right thing" without standing up Postgres.
    #[derive(Default)]
    struct CapturingEmitter {
        shaping: AtomicU32,
        gate_requests: AtomicU32,
        last_shaping_request: Mutex<Option<ShapingRequest>>,
        last_gate_id: Mutex<Option<String>>,
        /// When set to false, both emit methods return false to
        /// simulate a transient spine failure.
        accept: std::sync::atomic::AtomicBool,
    }
    impl CapturingEmitter {
        fn new() -> Self {
            Self {
                accept: std::sync::atomic::AtomicBool::new(true),
                ..Default::default()
            }
        }
    }
    impl GateEmitter for CapturingEmitter {
        fn emit_shaping_dispatched(&self, request: &ShapingRequest) -> bool {
            if !self.accept.load(Ordering::SeqCst) {
                return false;
            }
            self.shaping.fetch_add(1, Ordering::SeqCst);
            *self.last_shaping_request.lock().unwrap() = Some(request.clone());
            true
        }

        fn emit_gate_requested(&self, gate_id: &str, _request: &GateRequest) -> bool {
            if !self.accept.load(Ordering::SeqCst) {
                return false;
            }
            self.gate_requests.fetch_add(1, Ordering::SeqCst);
            *self.last_gate_id.lock().unwrap() = Some(gate_id.to_string());
            true
        }
    }

    fn make_artifact() -> Artifact {
        Artifact::new(Kind::Code, "x", "marvin", "forge", vec![])
    }

    fn make_workflow() -> Workflow {
        Workflow {
            workflow_id: "wf".into(),
            name: "t".into(),
            trigger: crate::core::workflow::TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "ai".into(),
            },
            stages: vec![],
            active: true,
            preset_id: None,
            install_id: None,
            created_by: None,
        }
    }

    #[test]
    fn agent_session_emits_once_then_polls_signal() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        // First tick: emit forge.shaping_dispatched, signal not yet
        // present → Pending.
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        assert_eq!(evaluator.emitter.shaping.load(Ordering::SeqCst), 1);

        // Second tick: still pending, but does NOT emit again.
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        assert_eq!(evaluator.emitter.shaping.load(Ordering::SeqCst), 1);

        // Signal lands (the workflow_signal_listener writes Success on
        // stiglab.session_completed): resolves Pass.
        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: AGENT_SESSION_SIGNAL.into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pass
        );
    }

    #[test]
    fn agent_session_does_not_mark_dispatched_on_emit_failure() {
        // A transient spine emit failure must not strand the artifact:
        // the gate stays "not dispatched yet" so the next tick retries.
        let cache = SignalCache::new();
        let emitter = CapturingEmitter::new();
        emitter
            .accept
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let evaluator = LiveGateEvaluator::new(cache, emitter, PendingVerdicts::new());
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        // Emitter rejected → not marked dispatched. The next tick can
        // retry under the budget.
        assert!(!evaluator.already_dispatched(artifact.artifact_id.as_str(), 0));
        // Flip the emitter back on: a follow-up tick succeeds.
        evaluator
            .emitter
            .accept
            .store(true, std::sync::atomic::Ordering::SeqCst);
        evaluator.reset_dispatch_budget();
        evaluator.evaluate(&artifact, &wf, 0, &gate);
        assert!(evaluator.already_dispatched(artifact.artifact_id.as_str(), 0));
    }

    #[test]
    fn external_check_resolves_on_ci_signal() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::ExternalCheck {
            check_name: "ci/test".into(),
        };

        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );

        // Red build → Fail.
        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: external_check_signal_kind("ci/test"),
                outcome: SignalOutcome::Failure("red".into()),
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Fail("red".into())
        );

        // Rerun → green → Pass.
        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: external_check_signal_kind("ci/test"),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pass
        );
    }

    #[test]
    fn governance_emits_request_then_claims_verdict_from_pending() {
        // First tick: emit forge.gate_requested + park gate_id.
        // Second tick (no verdict yet): still Pending, no second emit.
        // Third tick (verdict landed): map to Pass/Fail.
        let pending = PendingVerdicts::new();
        let evaluator =
            LiveGateEvaluator::new(SignalCache::new(), CapturingEmitter::new(), pending.clone());
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::Governance { gate_point: None };

        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        let gate_id = evaluator
            .emitter
            .last_gate_id
            .lock()
            .unwrap()
            .clone()
            .expect("emit_gate_requested should have captured an id");
        assert_eq!(evaluator.emitter.gate_requests.load(Ordering::SeqCst), 1);

        // No verdict yet → Pending, no second emit.
        evaluator.reset_dispatch_budget();
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        assert_eq!(evaluator.emitter.gate_requests.load(Ordering::SeqCst), 1);

        // Park an Allow verdict for this gate → Pass.
        pending.insert(&gate_id, GateVerdict::Allow);
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pass
        );
    }

    #[test]
    fn governance_maps_deny_verdict_to_fail() {
        let pending = PendingVerdicts::new();
        let evaluator =
            LiveGateEvaluator::new(SignalCache::new(), CapturingEmitter::new(), pending.clone());
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::Governance { gate_point: None };

        evaluator.evaluate(&artifact, &wf, 0, &gate);
        let gate_id = evaluator
            .emitter
            .last_gate_id
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        pending.insert(
            &gate_id,
            GateVerdict::Deny {
                reason: "nope".into(),
            },
        );
        match evaluator.evaluate(&artifact, &wf, 0, &gate) {
            GateOutcome::Fail(reason) => assert_eq!(reason, "nope"),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn manual_approval_only_resolves_on_matching_signal() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::ManualApproval {
            signal_kind: "pr_merged".into(),
        };

        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );

        // A different signal kind does NOT satisfy this gate.
        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: "dashboard_approve".into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );

        // Matching signal → pass.
        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: "pr_merged".into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pass
        );
    }

    #[test]
    fn dispatch_budget_caps_agent_session_per_tick() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::with_budget(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
            2,
        );
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        let a1 = Artifact::new(onsager_artifact::Kind::Code, "a1", "m", "forge", vec![]);
        let a2 = Artifact::new(onsager_artifact::Kind::Code, "a2", "m", "forge", vec![]);
        let a3 = Artifact::new(onsager_artifact::Kind::Code, "a3", "m", "forge", vec![]);

        evaluator.evaluate(&a1, &wf, 0, &gate);
        evaluator.evaluate(&a2, &wf, 0, &gate);
        evaluator.evaluate(&a3, &wf, 0, &gate);
        // Budget 2 → only 2 emits even though 3 artifacts asked.
        assert_eq!(evaluator.emitter.shaping.load(Ordering::SeqCst), 2);

        // Next tick: budget refills, a3 finally gets emitted.
        evaluator.reset_dispatch_budget();
        evaluator.evaluate(&a3, &wf, 0, &gate);
        assert_eq!(evaluator.emitter.shaping.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn on_stage_advanced_clears_agent_session_signal() {
        // After a stage advances, the next stage's agent-session gate
        // must not be satisfied by the prior stage's signal.
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();

        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: AGENT_SESSION_SIGNAL.into(),
                outcome: SignalOutcome::Success,
            },
        );
        assert!(cache
            .get(artifact.artifact_id.as_str(), AGENT_SESSION_SIGNAL)
            .is_some());

        evaluator.on_stage_advanced(&artifact.artifact_id, 0);
        assert!(cache
            .get(artifact.artifact_id.as_str(), AGENT_SESSION_SIGNAL)
            .is_none());
    }

    #[test]
    fn on_stage_advanced_clears_governance_in_flight() {
        // The governance in-flight map tracks which stage emitted which
        // gate_id; advancing the stage must drop that entry so a
        // revise cycle re-emits a fresh request.
        let evaluator = LiveGateEvaluator::new(
            SignalCache::new(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::Governance { gate_point: None };

        // Emit + park.
        evaluator.evaluate(&artifact, &wf, 0, &gate);
        assert!(evaluator
            .governance_in_flight
            .lock()
            .unwrap()
            .contains_key(&(artifact.artifact_id.as_str().to_string(), 0)));

        evaluator.on_stage_advanced(&artifact.artifact_id, 0);
        assert!(!evaluator
            .governance_in_flight
            .lock()
            .unwrap()
            .contains_key(&(artifact.artifact_id.as_str().to_string(), 0)));
    }

    #[test]
    fn agent_session_forwards_workflow_created_by_to_dispatch() {
        // Issue #156: forge populates `ShapingRequest.created_by` from
        // the workflow's owner so stiglab can decrypt that user's
        // `CLAUDE_CODE_OAUTH_TOKEN`. Without this, the spawned agent
        // boots with no auth and exits immediately. Now that the
        // dispatch flows through the spine, the assertion is on the
        // emitted request's `created_by`.
        let evaluator = LiveGateEvaluator::new(
            SignalCache::new(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = Workflow {
            workflow_id: "wf".into(),
            name: "t".into(),
            trigger: crate::core::workflow::TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "ai".into(),
            },
            stages: vec![],
            active: true,
            preset_id: None,
            install_id: None,
            created_by: Some("user_owner_42".into()),
        };
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        evaluator.evaluate(&artifact, &wf, 0, &gate);
        let captured = evaluator
            .emitter
            .last_shaping_request
            .lock()
            .unwrap()
            .clone()
            .expect("emit_shaping_dispatched should have captured a request");
        assert_eq!(captured.created_by.as_deref(), Some("user_owner_42"));
    }

    #[test]
    fn agent_session_resolves_fail_when_signal_is_failure() {
        // Issue #156: when stiglab.session_failed lands for an in-flight
        // workflow artifact, the workflow_signal_listener writes a
        // `Failure` outcome to the signal cache. The next tick must
        // surface it as `GateOutcome::Fail` so the artifact parks in
        // `workflow_parked_reason` instead of stalling at `Pending`
        // and re-dispatching forever.
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(
            cache.clone(),
            CapturingEmitter::new(),
            PendingVerdicts::new(),
        );
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        cache.push(
            artifact.artifact_id.as_str(),
            crate::core::signal_cache::Signal {
                kind: AGENT_SESSION_SIGNAL.into(),
                outcome: SignalOutcome::Failure("stdout closed without result event".into()),
            },
        );
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Fail("stdout closed without result event".into())
        );
        // And the gate did NOT emit in response to the failure — the
        // cache hit fires before the emit path.
        assert_eq!(evaluator.emitter.shaping.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn gate_point_parser_accepts_known_values() {
        assert_eq!(parse_gate_point("pre_dispatch"), GatePoint::PreDispatch);
        assert_eq!(
            parse_gate_point("consumer_routing"),
            GatePoint::ConsumerRouting
        );
        assert_eq!(parse_gate_point("tool_level"), GatePoint::ToolLevel);
        assert_eq!(
            parse_gate_point("state_transition"),
            GatePoint::StateTransition
        );
        // Unknown → fallback.
        assert_eq!(parse_gate_point("garbage"), GatePoint::StateTransition);
    }
}
