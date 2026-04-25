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

/// Dispatcher error code returned when forge couldn't even hand the
/// request to stiglab (network failure, 5xx on POST, polling timeout
/// before any session_id was assigned). Distinct from session-level
/// failures, which carry their own error codes and a populated
/// `session_id`. Mirrors the constant `HttpStiglabDispatcher` writes in
/// `cmd::serve` — kept as a string here to avoid forge's core depending
/// on the binary's adapter layer.
const DISPATCH_TRANSPORT_ERROR_CODE: &str = "dispatch_error";

/// Per-(artifact, stage) dispatch state. Held in the
/// [`LiveGateEvaluator::dispatched`] map.
#[derive(Debug, Clone)]
struct DispatchAttempt {
    /// Stable idempotency key reused across every retry for this
    /// (artifact, stage). Generated once on the first attempt and held
    /// until [`on_stage_advanced`] clears the entry.
    request_id: String,
    /// Whether stiglab accepted the request (POST returned a session_id
    /// and no `dispatch_error`). Once true we never resend — the signal
    /// listener is responsible for resolving the gate.
    posted: bool,
}

/// Did stiglab actually accept the request? True when there's no
/// transport-level error AND a session_id was returned. Distinguishes
/// "couldn't even POST" (retriable here) from "session ran and failed"
/// (the listener will write a Failure signal — retrying would create
/// duplicate sessions).
fn dispatch_was_accepted(result: &onsager_protocol::ShapingResult) -> bool {
    let is_transport_error = result
        .error
        .as_ref()
        .is_some_and(|e| e.code == DISPATCH_TRANSPORT_ERROR_CODE);
    !is_transport_error && !result.session_id.is_empty()
}

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
    /// Tracks per-(artifact_id, stage_index) dispatch state so retries
    /// can reuse the same idempotency key across ticks. `posted=true`
    /// means stiglab has accepted the request — we keep waiting for
    /// the signal listener and never resend. `posted=false` means we
    /// tried at least once and the dispatcher returned `dispatch_error`
    /// (transport-level failure) — next tick reuses the stored
    /// `request_id` so a retry that races a slow first POST collapses
    /// onto one stiglab session via `Idempotency-Key` dedup.
    dispatched: Mutex<HashMap<(String, u32), DispatchAttempt>>,
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

    /// Look up the existing attempt for this (artifact, stage), or
    /// create a fresh one with a new ULID `request_id`. Either way the
    /// returned `request_id` is stable across retries until the entry
    /// is cleared by [`on_stage_advanced`] — this is what makes
    /// stiglab's `Idempotency-Key` dedup work across forge retries.
    fn get_or_init_attempt(&self, artifact_id: &str, stage_index: u32) -> DispatchAttempt {
        let mut map = self.dispatched.lock().expect("dispatched map poisoned");
        map.entry((artifact_id.to_string(), stage_index))
            .or_insert_with(|| DispatchAttempt {
                request_id: ulid::Ulid::new().to_string(),
                posted: false,
            })
            .clone()
    }

    fn mark_posted(&self, artifact_id: &str, stage_index: u32) {
        let mut map = self.dispatched.lock().expect("dispatched map poisoned");
        if let Some(attempt) = map.get_mut(&(artifact_id.to_string(), stage_index)) {
            attempt.posted = true;
        }
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
        let attempt = self.get_or_init_attempt(artifact.artifact_id.as_str(), stage_index);
        if attempt.posted {
            // Already accepted by stiglab — just wait for the listener
            // to write the signal. Don't redispatch, even if the gate
            // is re-evaluated before the signal arrives.
            return GateOutcome::Pending;
        }
        if !self.try_consume_dispatch() {
            tracing::debug!(
                artifact_id = %artifact.artifact_id,
                stage_index,
                "workflow gate: agent-session dispatch budget exhausted this tick"
            );
            return GateOutcome::Pending;
        }
        let request = onsager_protocol::ShapingRequest {
            request_id: attempt.request_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            target_version: artifact.current_version + 1,
            shaping_intent: shaping_intent.clone(),
            inputs: vec![],
            constraints: vec![],
            deadline: None,
        };
        // Distinguish "stiglab never accepted the POST" (dispatch_error,
        // empty session_id — transport failure) from "stiglab accepted
        // and the session ran but failed" (other error codes, populated
        // session_id). The former is the only retriable case here; the
        // latter is the session listener's job to surface as a Failure
        // signal so the gate resolves Fail. Retrying past a real session
        // failure would create duplicate sessions for the same artifact.
        //
        // Idempotency: the request reuses `attempt.request_id` across
        // retries. `HttpStiglabDispatcher` sends it as the
        // `Idempotency-Key` header, and stiglab's
        // `find_session_by_idempotency_key` collapses concurrent
        // attempts onto a single session.
        let result = self.stiglab.dispatch(&request);
        if dispatch_was_accepted(&result) {
            self.mark_posted(artifact.artifact_id.as_str(), stage_index);
        } else {
            tracing::warn!(
                artifact_id = %artifact.artifact_id,
                stage_index,
                error = ?result.error,
                "workflow gate: agent-session dispatch failed; will retry next tick \
                 with the same idempotency key"
            );
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

    /// Dispatcher that always returns a `dispatch_error` ShapingResult,
    /// simulating stiglab being unreachable / 503 from no available
    /// runner nodes. Mirrors the shape `HttpStiglabDispatcher` returns
    /// from `cmd::serve` when the POST or polling fails. Captures the
    /// request_id of every attempt so the test can assert idempotency.
    struct FailingStiglab {
        dispatches: AtomicU32,
        request_ids: Mutex<Vec<String>>,
    }
    impl FailingStiglab {
        fn new() -> Self {
            Self {
                dispatches: AtomicU32::new(0),
                request_ids: Mutex::new(Vec::new()),
            }
        }
    }
    impl StiglabDispatcher for FailingStiglab {
        fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            self.request_ids
                .lock()
                .unwrap()
                .push(req.request_id.clone());
            ShapingResult {
                request_id: req.request_id.clone(),
                outcome: ShapingOutcome::Failed,
                content_ref: None,
                change_summary: String::new(),
                quality_signals: vec![],
                session_id: String::new(),
                duration_ms: 0,
                error: Some(onsager_protocol::ErrorDetail {
                    code: "dispatch_error".into(),
                    message: "stiglab unreachable".into(),
                    retriable: Some(true),
                }),
            }
        }
    }

    /// Dispatcher that simulates "session was accepted by stiglab and
    /// then ran and failed." Returns a non-empty `session_id` and an
    /// error whose code is NOT `dispatch_error` — distinct from a
    /// transport failure. The gate must NOT retry this case (doing so
    /// would create duplicate sessions); the signal listener is
    /// responsible for surfacing the failure as a `Fail` outcome.
    struct SessionFailedStiglab {
        dispatches: AtomicU32,
    }
    impl SessionFailedStiglab {
        fn new() -> Self {
            Self {
                dispatches: AtomicU32::new(0),
            }
        }
    }
    impl StiglabDispatcher for SessionFailedStiglab {
        fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            ShapingResult {
                request_id: req.request_id.clone(),
                outcome: ShapingOutcome::Failed,
                content_ref: None,
                change_summary: String::new(),
                quality_signals: vec![],
                session_id: "sess_already_ran".into(),
                duration_ms: 1234,
                error: Some(onsager_protocol::ErrorDetail {
                    code: "agent_failure".into(),
                    message: "agent exited 1".into(),
                    retriable: Some(false),
                }),
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
    fn agent_session_retries_dispatch_after_failure() {
        // Regression: a stiglab-unreachable dispatch must NOT be marked
        // as already-dispatched. Otherwise the artifact parks at stage 0
        // forever, even after stiglab recovers (issue from the
        // claude/debug-trigger session). The gate evaluator must keep
        // retrying every tick until dispatch succeeds.
        let cache = SignalCache::new();
        let stiglab = FailingStiglab::new();
        let evaluator = LiveGateEvaluator::new(cache, stiglab, AllowSynodic);
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        for _ in 0..3 {
            evaluator.reset_dispatch_budget();
            assert_eq!(
                evaluator.evaluate(&artifact, &wf, 0, &gate),
                GateOutcome::Pending
            );
        }
        assert_eq!(
            evaluator.stiglab.dispatches.load(Ordering::SeqCst),
            3,
            "failed dispatches should retry on every tick, not be silently marked done"
        );

        // Idempotency: every retry MUST send the same request_id so
        // stiglab's Idempotency-Key dedup collapses concurrent attempts
        // onto one session. A fresh ULID per retry would create N
        // sessions for the same artifact-stage.
        let ids = evaluator.stiglab.request_ids.lock().unwrap().clone();
        assert_eq!(ids.len(), 3);
        assert!(
            ids.iter().all(|id| id == &ids[0]),
            "request_id must be stable across retries; got {ids:?}"
        );
    }

    #[test]
    fn agent_session_does_not_retry_on_session_failure() {
        // Regression for a Copilot review point: a session that stiglab
        // accepted, ran, and reported failed must NOT trigger another
        // dispatch. The signal listener is responsible for surfacing
        // session failure as a `Fail` outcome on a later tick.
        let cache = SignalCache::new();
        let stiglab = SessionFailedStiglab::new();
        let evaluator = LiveGateEvaluator::new(cache, stiglab, AllowSynodic);
        let artifact = make_artifact();
        let wf = make_workflow();
        let gate = GateSpec::AgentSession {
            shaping_intent: serde_json::Value::Null,
        };

        for _ in 0..3 {
            evaluator.reset_dispatch_budget();
            assert_eq!(
                evaluator.evaluate(&artifact, &wf, 0, &gate),
                GateOutcome::Pending
            );
        }
        assert_eq!(
            evaluator.stiglab.dispatches.load(Ordering::SeqCst),
            1,
            "a stiglab-accepted session that failed at runtime must not be redispatched"
        );
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
