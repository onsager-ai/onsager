//! Live gate evaluator implementations (issue #80).
//!
//! Wires the four gate kinds to their production-time backends:
//!
//! - `agent-session` — dispatches via the existing [`StiglabDispatcher`]
//!   on first observation, then resolves when the matching
//!   `stiglab.session_completed` event lands in the [`SignalCache`].
//! - `external-check` — consumes the cache, where the GitHub CI-event
//!   listener has written a `ci:<check_name>` signal.
//! - `governance` — calls the existing [`SynodicGate`] with a
//!   `StateTransition` request shape.
//! - `manual-approval` — consumes the cache for the
//!   `signal_kind` declared on the gate (e.g. `pr_merged`,
//!   `dashboard_approve`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use onsager_artifact::Artifact;
use onsager_protocol::{GateContext, GateRequest, GateVerdict, ProposedAction};
use onsager_spine::factory_event::GatePoint;

use super::pipeline::{StiglabDispatcher, SynodicGate};
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
/// Stiglab under the Forge write lock (issue #80 copilot-review).
pub const DEFAULT_DISPATCH_BUDGET_PER_TICK: u32 = 4;

/// Live gate evaluator that backs the stage runner in production.
pub struct LiveGateEvaluator<S, G>
where
    S: StiglabDispatcher,
    G: SynodicGate,
{
    signals: SignalCache,
    /// Dispatcher for `agent-session` gates. Gates that haven't been
    /// dispatched yet get a one-shot dispatch; subsequent ticks just poll
    /// the signal cache.
    stiglab: S,
    /// Synodic gate used for `governance` gate kinds.
    synodic: G,
    /// Tracks which (artifact_id, stage_index) pairs already had their
    /// agent-session dispatched so we don't fire the same session
    /// multiple times across ticks.
    dispatched: Mutex<HashMap<(String, u32), ()>>,
    /// Remaining agent-session dispatches allowed this tick. The caller
    /// resets this via [`reset_dispatch_budget`] once per pass so a
    /// burst of new workflow artifacts can't synchronously hammer
    /// Stiglab under the Forge write lock.
    dispatch_budget: AtomicU32,
    /// Budget ceiling refilled by [`reset_dispatch_budget`].
    dispatch_budget_per_tick: u32,
}

impl<S, G> LiveGateEvaluator<S, G>
where
    S: StiglabDispatcher,
    G: SynodicGate,
{
    pub fn new(signals: SignalCache, stiglab: S, synodic: G) -> Self {
        Self::with_budget(signals, stiglab, synodic, DEFAULT_DISPATCH_BUDGET_PER_TICK)
    }

    pub fn with_budget(
        signals: SignalCache,
        stiglab: S,
        synodic: G,
        dispatch_budget_per_tick: u32,
    ) -> Self {
        Self {
            signals,
            stiglab,
            synodic,
            dispatched: Mutex::new(HashMap::new()),
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

    fn mark_dispatched(&self, artifact_id: &str, stage_index: u32) {
        let mut map = self.dispatched.lock().expect("dispatched map poisoned");
        map.insert((artifact_id.to_string(), stage_index), ());
    }

    fn evaluate_agent_session(
        &self,
        artifact: &Artifact,
        stage_index: u32,
        shaping_intent: &serde_json::Value,
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

        // First observation: dispatch a shaping request. Subsequent ticks
        // will find the signal (set by the session listener) and resolve.
        //
        // Budget-gated so a burst of new workflow artifacts can't fire
        // N synchronous Stiglab requests under the Forge write lock. If
        // the budget is exhausted this tick, return Pending and retry
        // next tick — the artifact stays at this stage.
        if !self.already_dispatched(artifact.artifact_id.as_str(), stage_index) {
            if !self.try_consume_dispatch() {
                tracing::debug!(
                    artifact_id = %artifact.artifact_id,
                    stage_index,
                    "workflow gate: agent-session dispatch budget exhausted this tick"
                );
                return GateOutcome::Pending;
            }
            let request = onsager_protocol::ShapingRequest {
                request_id: ulid::Ulid::new().to_string(),
                artifact_id: artifact.artifact_id.clone(),
                target_version: artifact.current_version + 1,
                shaping_intent: shaping_intent.clone(),
                inputs: vec![],
                constraints: vec![],
                deadline: None,
            };
            // The dispatcher call may complete synchronously (legacy
            // stiglab path) and return immediately; the session listener
            // is still responsible for writing the signal so the gate
            // resolves on a subsequent tick.
            let _ = self.stiglab.dispatch(&request);
            self.mark_dispatched(artifact.artifact_id.as_str(), stage_index);
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

    fn evaluate_governance(&self, artifact: &Artifact, gate_point: &Option<String>) -> GateOutcome {
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

        match self.synodic.evaluate(&request) {
            GateVerdict::Allow | GateVerdict::Modify { .. } => GateOutcome::Pass,
            GateVerdict::Deny { reason } => GateOutcome::Fail(reason),
            // Escalate: park non-blockingly (forge invariant #5) — we
            // report Pending so the artifact stays at this stage until
            // the escalation resolves.
            GateVerdict::Escalate { .. } => GateOutcome::Pending,
        }
    }

    fn evaluate_manual_approval(&self, artifact: &Artifact, signal_kind: &str) -> GateOutcome {
        match self.signals.get(artifact.artifact_id.as_str(), signal_kind) {
            Some(SignalOutcome::Success) => GateOutcome::Pass,
            Some(SignalOutcome::Failure(reason)) => GateOutcome::Fail(reason),
            None => GateOutcome::Pending,
        }
    }
}

impl<S, G> super::stage_runner::GateEvaluator for LiveGateEvaluator<S, G>
where
    S: StiglabDispatcher,
    G: SynodicGate,
{
    fn evaluate(
        &self,
        artifact: &Artifact,
        _workflow: &Workflow,
        stage_index: u32,
        gate: &GateSpec,
    ) -> GateOutcome {
        match gate {
            GateSpec::AgentSession { shaping_intent } => {
                self.evaluate_agent_session(artifact, stage_index, shaping_intent)
            }
            GateSpec::ExternalCheck { check_name } => {
                self.evaluate_external_check(artifact, check_name)
            }
            GateSpec::Governance { gate_point } => self.evaluate_governance(artifact, gate_point),
            GateSpec::ManualApproval { signal_kind } => {
                self.evaluate_manual_approval(artifact, signal_kind)
            }
        }
    }

    fn on_stage_advanced(&self, artifact_id: &onsager_artifact::ArtifactId, stage_index: u32) {
        // Clear the agent-session signal so a completed session can't
        // satisfy a later stage's `agent-session` gate for the same
        // artifact. Also drop the "already dispatched" marker for this
        // stage so a revise cycle (if any) redispatches.
        self.signals
            .clear(artifact_id.as_str(), AGENT_SESSION_SIGNAL);
        let mut map = self.dispatched.lock().expect("dispatched map poisoned");
        map.remove(&(artifact_id.as_str().to_string(), stage_index));
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
    use crate::core::pipeline::{StiglabDispatcher, SynodicGate};
    use crate::core::stage_runner::GateEvaluator;
    use onsager_artifact::Kind;
    use onsager_protocol::{ShapingRequest, ShapingResult};
    use onsager_spine::factory_event::ShapingOutcome;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingStiglab {
        dispatches: AtomicU32,
    }
    impl CountingStiglab {
        fn new() -> Self {
            Self {
                dispatches: AtomicU32::new(0),
            }
        }
    }
    impl StiglabDispatcher for CountingStiglab {
        fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            ShapingResult {
                request_id: req.request_id.clone(),
                outcome: ShapingOutcome::Completed,
                content_ref: None,
                change_summary: String::new(),
                quality_signals: vec![],
                session_id: "sess".into(),
                duration_ms: 0,
                error: None,
            }
        }
    }

    struct AllowSynodic;
    impl SynodicGate for AllowSynodic {
        fn evaluate(&self, _req: &GateRequest) -> GateVerdict {
            GateVerdict::Allow
        }
    }

    struct DenySynodic;
    impl SynodicGate for DenySynodic {
        fn evaluate(&self, _req: &GateRequest) -> GateVerdict {
            GateVerdict::Deny {
                reason: "nope".into(),
            }
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
            workspace_install_ref: None,
        }
    }

    #[test]
    fn agent_session_dispatches_once_then_polls() {
        let cache = SignalCache::new();
        let stiglab = CountingStiglab::new();
        let evaluator = LiveGateEvaluator::new(cache.clone(), stiglab, AllowSynodic);
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        // First tick: dispatches, signal not yet present → Pending.
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        assert_eq!(evaluator.stiglab.dispatches.load(Ordering::SeqCst), 1);

        // Second tick: still pending, but does NOT dispatch again.
        assert_eq!(
            evaluator.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pending
        );
        assert_eq!(evaluator.stiglab.dispatches.load(Ordering::SeqCst), 1);

        // Signal lands: resolves Pass.
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
    fn external_check_resolves_on_ci_signal() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(cache.clone(), CountingStiglab::new(), AllowSynodic);
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
    fn governance_maps_synodic_verdict() {
        let allow_eval =
            LiveGateEvaluator::new(SignalCache::new(), CountingStiglab::new(), AllowSynodic);
        let deny_eval =
            LiveGateEvaluator::new(SignalCache::new(), CountingStiglab::new(), DenySynodic);
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::Governance { gate_point: None };

        assert_eq!(
            allow_eval.evaluate(&artifact, &wf, 0, &gate),
            GateOutcome::Pass
        );
        match deny_eval.evaluate(&artifact, &wf, 0, &gate) {
            GateOutcome::Fail(_) => (),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn manual_approval_only_resolves_on_matching_signal() {
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(cache.clone(), CountingStiglab::new(), AllowSynodic);
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::ManualApproval {
            signal_kind: "pr_merged".into(),
        };

        // No signal → pending.
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
        let stiglab = CountingStiglab::new();
        let evaluator = LiveGateEvaluator::with_budget(cache.clone(), stiglab, AllowSynodic, 2);
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        // Three distinct artifacts competing for the budget.
        let a1 = Artifact::new(onsager_artifact::Kind::Code, "a1", "m", "forge", vec![]);
        let a2 = Artifact::new(onsager_artifact::Kind::Code, "a2", "m", "forge", vec![]);
        let a3 = Artifact::new(onsager_artifact::Kind::Code, "a3", "m", "forge", vec![]);

        evaluator.evaluate(&a1, &wf, 0, &gate);
        evaluator.evaluate(&a2, &wf, 0, &gate);
        evaluator.evaluate(&a3, &wf, 0, &gate);
        // Budget 2 → only 2 dispatches even though 3 artifacts asked.
        assert_eq!(evaluator.stiglab.dispatches.load(Ordering::SeqCst), 2);

        // Next tick: budget refills, a3 finally gets dispatched.
        evaluator.reset_dispatch_budget();
        evaluator.evaluate(&a3, &wf, 0, &gate);
        assert_eq!(evaluator.stiglab.dispatches.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn on_stage_advanced_clears_agent_session_signal() {
        // After a stage advances, the next stage's agent-session gate
        // must not be satisfied by the prior stage's signal.
        let cache = SignalCache::new();
        let evaluator = LiveGateEvaluator::new(cache.clone(), CountingStiglab::new(), AllowSynodic);
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

        // Runner signals advance.
        use crate::core::stage_runner::GateEvaluator;
        evaluator.on_stage_advanced(&artifact.artifact_id, 0);
        assert!(cache
            .get(artifact.artifact_id.as_str(), AGENT_SESSION_SIGNAL)
            .is_none());
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
