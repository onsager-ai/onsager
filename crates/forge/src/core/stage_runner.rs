//! Workflow stage runner (issue #80).
//!
//! The stage runner walks each workflow-tagged artifact through its declared
//! stage chain in strict declared order. Per tick, it:
//!
//! 1. Finds every active workflow.
//! 2. For each artifact tagged with that workflow, evaluates the gates on
//!    the artifact's current stage.
//! 3. When all gates pass, advances the artifact to the next stage (or
//!    marks the workflow complete when past the last stage).
//! 4. When any gate fails, parks the artifact in `UnderReview` with the
//!    failure reason. A later re-evaluation of the same gate can clear
//!    the park and let the artifact advance.
//!
//! Gate evaluation is injected via [`GateEvaluator`] so the runner is pure:
//! deterministic tests hand it a mock, production wires it to the synodic
//! HTTP gate + signal cache + stiglab dispatcher.

use std::collections::HashMap;

use onsager_artifact::{Artifact, ArtifactId, ArtifactState};

use super::artifact_store::ArtifactStore;
use super::workflow::{GateOutcome, GateSpec, Workflow};

/// Event emitted by a single stage-runner tick. These translate 1:1 to
/// `stage.*` factory events on the spine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageEvent {
    /// The artifact transitioned into a new stage.
    StageEntered {
        artifact_id: String,
        workflow_id: String,
        stage_index: u32,
        stage_name: String,
    },
    /// A gate on the current stage resolved successfully.
    GatePassed {
        artifact_id: String,
        workflow_id: String,
        stage_index: u32,
        gate_kind: String,
    },
    /// A gate on the current stage failed. Artifact is parked.
    GateFailed {
        artifact_id: String,
        workflow_id: String,
        stage_index: u32,
        gate_kind: String,
        reason: String,
    },
    /// All gates on the stage resolved; artifact advances.
    StageAdvanced {
        artifact_id: String,
        workflow_id: String,
        from_stage_index: u32,
        to_stage_index: Option<u32>,
    },
}

/// Abstraction over gate evaluation. Production wires this to the signal
/// cache + synodic gate + stiglab dispatcher; tests hand it a mock.
pub trait GateEvaluator {
    /// Evaluate a single gate against an artifact at its current stage.
    fn evaluate(
        &self,
        artifact: &Artifact,
        workflow: &Workflow,
        stage_index: u32,
        gate: &GateSpec,
    ) -> GateOutcome;

    /// Called by the runner when an artifact advances past a stage.
    /// Implementations use this to clear any stage-scoped state (e.g.
    /// the agent-session signal) so a stale signal can't satisfy a
    /// later stage's gate for the same artifact.
    ///
    /// Default no-op: evaluators without per-stage state do nothing.
    fn on_stage_advanced(&self, _artifact_id: &ArtifactId, _stage_index: u32) {}
}

/// Advance every workflow-tagged artifact by one stage runner pass.
///
/// Returns the events emitted so the caller can persist them to the spine.
/// Inactive workflows are still evaluated for *in-flight* artifacts (so an
/// operator disabling a workflow mid-run lets existing artifacts finish),
/// but the trigger subscriber won't register new ones against an inactive
/// workflow.
pub fn advance_workflow_artifacts<E: GateEvaluator>(
    workflows: &HashMap<String, Workflow>,
    store: &mut ArtifactStore,
    evaluator: &E,
) -> Vec<StageEvent> {
    let mut events = Vec::new();

    // Snapshot artifact ids up-front so we can mutate the store as we go.
    let targets: Vec<(String, String, u32)> = store
        .active_artifacts()
        .iter()
        .filter_map(|a| {
            let wf_id = a.workflow_id.clone()?;
            let idx = a.current_stage_index?;
            Some((a.artifact_id.as_str().to_string(), wf_id, idx))
        })
        .collect();

    for (artifact_id, workflow_id, stage_index) in targets {
        let Some(workflow) = workflows.get(&workflow_id) else {
            // Workflow was deleted out from under an in-flight artifact.
            // Leave the artifact alone — a future reconcile pass either
            // restores the workflow or archives the orphan explicitly.
            tracing::warn!(
                artifact_id = %artifact_id,
                workflow_id = %workflow_id,
                "stage runner: workflow missing for in-flight artifact"
            );
            continue;
        };

        advance_single_artifact(
            workflow,
            stage_index,
            &ArtifactId::new(&artifact_id),
            store,
            evaluator,
            &mut events,
        );
    }

    events
}

fn advance_single_artifact<E: GateEvaluator>(
    workflow: &Workflow,
    stage_index: u32,
    artifact_id: &ArtifactId,
    store: &mut ArtifactStore,
    evaluator: &E,
    events: &mut Vec<StageEvent>,
) {
    let stage = match workflow.stage(stage_index as usize) {
        Some(s) => s.clone(),
        None => {
            // Past the end of the chain — workflow was likely edited to
            // remove/reorder stages while this artifact was in-flight.
            // Park it with a clear reason and drop the stage tag so the
            // runner stops picking it up and an operator can reconcile
            // explicitly (issue #80 copilot-review).
            tracing::warn!(
                artifact_id = %artifact_id,
                workflow_id = %workflow.workflow_id,
                stage_index,
                "stage runner: artifact stage index is out of bounds for workflow; parking"
            );
            park_artifact(
                store,
                artifact_id,
                format!(
                    "stage_index {stage_index} out of bounds for workflow {}",
                    workflow.workflow_id
                ),
            );
            set_stage_index(store, artifact_id, None);
            return;
        }
    };

    // Evaluate every gate. Any pending → wait. Any fail → park + record
    // failure. All pass → advance.
    let artifact_snapshot = match store.get(artifact_id) {
        Some(a) => a.clone(),
        None => return,
    };

    let had_parked_reason = artifact_snapshot.workflow_parked_reason.is_some();

    let mut any_pending = false;
    let mut gate_failures: Vec<(String, String)> = Vec::new();
    let mut gate_passes: Vec<String> = Vec::new();

    for gate in &stage.gates {
        match evaluator.evaluate(&artifact_snapshot, workflow, stage_index, gate) {
            GateOutcome::Pass => gate_passes.push(gate.kind_tag().to_string()),
            GateOutcome::Fail(reason) => {
                gate_failures.push((gate.kind_tag().to_string(), reason));
            }
            GateOutcome::Pending => any_pending = true,
        }
    }

    if !gate_failures.is_empty() {
        // Park in UnderReview with the combined failure reason. Only emit
        // a GateFailed event per gate if the park reason is newly being
        // set (or would change) — otherwise repeated ticks over the same
        // failing condition would spam identical events.
        let parked_reason = gate_failures
            .iter()
            .map(|(k, r)| format!("{k}: {r}"))
            .collect::<Vec<_>>()
            .join("; ");
        let existing_reason = store
            .get(artifact_id)
            .and_then(|a| a.workflow_parked_reason.clone());
        if existing_reason.as_deref() != Some(parked_reason.as_str()) {
            for (gate_kind, reason) in gate_failures {
                events.push(StageEvent::GateFailed {
                    artifact_id: artifact_id.as_str().to_string(),
                    workflow_id: workflow.workflow_id.clone(),
                    stage_index,
                    gate_kind,
                    reason,
                });
            }
            park_artifact(store, artifact_id, parked_reason);
        }
        return;
    }

    if any_pending {
        // Still waiting on at least one gate. If the failing condition
        // that previously parked this artifact has now cleared, drop the
        // stale reason so the dashboard reflects reality even before the
        // last gate flips (issue #80 copilot-review).
        if had_parked_reason {
            clear_parked_reason(store, artifact_id);
        }
        return;
    }

    // All gates passed — advance the artifact to the next stage. Only now
    // do we emit `GatePassed` events — otherwise every tick where *some*
    // gates pass but others are pending would spam the stream.
    for gate_kind in gate_passes {
        events.push(StageEvent::GatePassed {
            artifact_id: artifact_id.as_str().to_string(),
            workflow_id: workflow.workflow_id.clone(),
            stage_index,
            gate_kind,
        });
    }

    let next_index = stage_index + 1;
    let next_stage = workflow.stage(next_index as usize).cloned();

    // When advancing, clear any parked reason from a prior failed attempt
    // and notify the evaluator so it can drop stage-scoped state (e.g.
    // the agent-session signal).
    clear_parked_reason(store, artifact_id);
    evaluator.on_stage_advanced(artifact_id, stage_index);

    if let Some(ref stage_after) = next_stage {
        // Transition artifact state if the next stage declares one.
        if let Some(target_state) = stage_after.target_state {
            apply_state_change(store, artifact_id, target_state);
        }
        set_stage_index(store, artifact_id, Some(next_index));
        events.push(StageEvent::StageAdvanced {
            artifact_id: artifact_id.as_str().to_string(),
            workflow_id: workflow.workflow_id.clone(),
            from_stage_index: stage_index,
            to_stage_index: Some(next_index),
        });
        events.push(StageEvent::StageEntered {
            artifact_id: artifact_id.as_str().to_string(),
            workflow_id: workflow.workflow_id.clone(),
            stage_index: next_index,
            stage_name: stage_after.name.clone(),
        });
    } else {
        // Ran past the last stage — this is workflow completion. Leave
        // the artifact in its current state (typically Released, set by
        // the final stage) but clear the stage_index so the runner stops
        // picking it up.
        set_stage_index(store, artifact_id, None);
        events.push(StageEvent::StageAdvanced {
            artifact_id: artifact_id.as_str().to_string(),
            workflow_id: workflow.workflow_id.clone(),
            from_stage_index: stage_index,
            to_stage_index: None,
        });
    }
}

fn park_artifact(store: &mut ArtifactStore, id: &ArtifactId, reason: String) {
    if let Some(artifact) = store.get_mut(id) {
        artifact.workflow_parked_reason = Some(reason);
        if artifact.state != ArtifactState::UnderReview
            && artifact.state.can_transition_to(ArtifactState::UnderReview)
        {
            artifact.state = ArtifactState::UnderReview;
        }
    }
}

fn clear_parked_reason(store: &mut ArtifactStore, id: &ArtifactId) {
    if let Some(artifact) = store.get_mut(id) {
        artifact.workflow_parked_reason = None;
    }
}

fn apply_state_change(store: &mut ArtifactStore, id: &ArtifactId, target: ArtifactState) {
    if let Some(artifact) = store.get_mut(id) {
        if artifact.state != target && artifact.state.can_transition_to(target) {
            artifact.state = target;
        }
    }
}

fn set_stage_index(store: &mut ArtifactStore, id: &ArtifactId, index: Option<u32>) {
    if let Some(artifact) = store.get_mut(id) {
        artifact.current_stage_index = index;
    }
}

/// Entry point: register a brand-new workflow-tagged artifact at stage 0.
/// Used by the trigger subscriber when a `trigger.fired` event is handled.
pub fn enter_workflow(
    store: &mut ArtifactStore,
    artifact_id: &ArtifactId,
    workflow: &Workflow,
) -> Option<StageEvent> {
    let first_stage = workflow.stage(0)?;

    if let Some(artifact) = store.get_mut(artifact_id) {
        artifact.workflow_id = Some(workflow.workflow_id.clone());
        artifact.current_stage_index = Some(0);
        artifact.workflow_parked_reason = None;
        if let Some(target) = first_stage.target_state {
            if artifact.state != target && artifact.state.can_transition_to(target) {
                artifact.state = target;
            }
        }
    }

    Some(StageEvent::StageEntered {
        artifact_id: artifact_id.as_str().to_string(),
        workflow_id: workflow.workflow_id.clone(),
        stage_index: 0,
        stage_name: first_stage.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workflow::{StageSpec, TriggerSpec};
    use onsager_artifact::Kind;

    fn make_workflow(id: &str, stages: Vec<StageSpec>) -> Workflow {
        Workflow {
            workflow_id: id.into(),
            name: "test".into(),
            trigger: TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "ai".into(),
            },
            stages,
            active: true,
            workspace_id: "ws_test".into(),
            preset_id: None,
            install_id: None,
            created_by: None,
        }
    }

    fn make_stage(name: &str, target: Option<ArtifactState>, gates: Vec<GateSpec>) -> StageSpec {
        StageSpec {
            name: name.into(),
            target_state: target,
            gates,
            params: serde_json::Value::Null,
        }
    }

    /// Mock evaluator with hand-scripted gate outcomes keyed by (artifact, gate_kind_tag).
    struct MockEvaluator {
        outcomes: HashMap<(String, String), GateOutcome>,
    }

    impl MockEvaluator {
        fn new() -> Self {
            Self {
                outcomes: HashMap::new(),
            }
        }

        fn set(&mut self, artifact_id: &str, gate_kind: &str, outcome: GateOutcome) {
            self.outcomes
                .insert((artifact_id.to_string(), gate_kind.to_string()), outcome);
        }
    }

    impl GateEvaluator for MockEvaluator {
        fn evaluate(
            &self,
            artifact: &Artifact,
            _workflow: &Workflow,
            _stage_index: u32,
            gate: &GateSpec,
        ) -> GateOutcome {
            self.outcomes
                .get(&(
                    artifact.artifact_id.as_str().to_string(),
                    gate.kind_tag().to_string(),
                ))
                .cloned()
                .unwrap_or(GateOutcome::Pending)
        }
    }

    fn enroll(store: &mut ArtifactStore, workflow: &Workflow, name: &str) -> ArtifactId {
        let id = store.register(Kind::Custom("github-issue".into()), name, "marvin");
        enter_workflow(store, &id, workflow);
        id
    }

    #[test]
    fn enter_workflow_sets_stage_and_state() {
        let wf = make_workflow(
            "wf_1",
            vec![make_stage(
                "implement",
                Some(ArtifactState::InProgress),
                vec![],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "x");
        let art = store.get(&id).unwrap();
        assert_eq!(art.workflow_id.as_deref(), Some("wf_1"));
        assert_eq!(art.current_stage_index, Some(0));
        assert_eq!(art.state, ArtifactState::InProgress);
    }

    #[test]
    fn runner_advances_through_strict_declared_order() {
        // Three stages, all gates pre-set to Pass. The runner must walk
        // them in order 0 → 1 → 2, never skipping.
        let wf = make_workflow(
            "wf_order",
            vec![
                make_stage(
                    "s0",
                    Some(ArtifactState::InProgress),
                    vec![GateSpec::ManualApproval {
                        signal_kind: "s0".into(),
                    }],
                ),
                make_stage(
                    "s1",
                    Some(ArtifactState::UnderReview),
                    vec![GateSpec::ManualApproval {
                        signal_kind: "s1".into(),
                    }],
                ),
                make_stage(
                    "s2",
                    Some(ArtifactState::Released),
                    vec![GateSpec::ManualApproval {
                        signal_kind: "s2".into(),
                    }],
                ),
            ],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "ordered");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf.clone());

        // Tick 1: only s0 passes. Artifact should land on stage 1.
        let mut eval = MockEvaluator::new();
        eval.set(id.as_str(), "manual_approval", GateOutcome::Pass);
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(1));
        assert_eq!(store.get(&id).unwrap().state, ArtifactState::UnderReview);

        // Tick 2: s1 passes. Advances to stage 2.
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(2));

        // Tick 3: s2 passes. Advances past the end → stage_index cleared.
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, None);
        assert_eq!(store.get(&id).unwrap().state, ArtifactState::Released);
    }

    #[test]
    fn runner_blocks_on_pending_gate() {
        let wf = make_workflow(
            "wf_pending",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![
                    GateSpec::ManualApproval {
                        signal_kind: "ci".into(),
                    },
                    GateSpec::ManualApproval {
                        signal_kind: "merge".into(),
                    },
                ],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "pending");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        // Both gates are pending (evaluator returns Pending by default).
        let eval = MockEvaluator::new();
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(0));
    }

    #[test]
    fn runner_parks_artifact_on_gate_failure() {
        let wf = make_workflow(
            "wf_fail",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![GateSpec::ExternalCheck {
                    check_name: "ci".into(),
                }],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "failing");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        eval.set(
            id.as_str(),
            "external_check",
            GateOutcome::Fail("red build".into()),
        );

        let events = advance_workflow_artifacts(&workflows, &mut store, &eval);
        let art = store.get(&id).unwrap();
        assert_eq!(art.state, ArtifactState::UnderReview);
        assert!(art.workflow_parked_reason.is_some());
        assert!(events.iter().any(|e| matches!(
            e,
            StageEvent::GateFailed { gate_kind, .. } if gate_kind == "external_check"
        )));
        // Artifact stays on stage 0 — failure parks, doesn't advance.
        assert_eq!(art.current_stage_index, Some(0));
    }

    #[test]
    fn runner_clears_park_when_gate_recovers() {
        let wf = make_workflow(
            "wf_recover",
            vec![
                make_stage(
                    "s0",
                    Some(ArtifactState::InProgress),
                    vec![GateSpec::ExternalCheck {
                        check_name: "ci".into(),
                    }],
                ),
                make_stage("s1", Some(ArtifactState::Released), vec![]),
            ],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "recovering");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        eval.set(
            id.as_str(),
            "external_check",
            GateOutcome::Fail("red".into()),
        );
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert!(store.get(&id).unwrap().workflow_parked_reason.is_some());

        // CI re-run goes green — runner should clear park and advance.
        eval.set(id.as_str(), "external_check", GateOutcome::Pass);
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        let art = store.get(&id).unwrap();
        assert_eq!(art.current_stage_index, Some(1));
        assert!(art.workflow_parked_reason.is_none());
        assert_eq!(art.state, ArtifactState::Released);
    }

    #[test]
    fn runner_waits_for_all_gates_to_pass() {
        let wf = make_workflow(
            "wf_multi",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![
                    GateSpec::AgentSession {
                        shaping_intent: serde_json::Value::Null,
                    },
                    GateSpec::ManualApproval {
                        signal_kind: "approve".into(),
                    },
                ],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "multigate");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        // Only one gate passes — still pending.
        let mut eval = MockEvaluator::new();
        eval.set(id.as_str(), "agent_session", GateOutcome::Pass);
        // manual_approval is pending by default.
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(0));

        // Second gate resolves — advances past end of chain.
        eval.set(id.as_str(), "manual_approval", GateOutcome::Pass);
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, None);
    }

    #[test]
    fn runner_emits_stage_entered_on_advance() {
        let wf = make_workflow(
            "wf_events",
            vec![
                make_stage(
                    "first",
                    Some(ArtifactState::InProgress),
                    vec![GateSpec::ManualApproval {
                        signal_kind: "a".into(),
                    }],
                ),
                make_stage("second", Some(ArtifactState::UnderReview), vec![]),
            ],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "eventful");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        eval.set(id.as_str(), "manual_approval", GateOutcome::Pass);

        let events = advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert!(events.iter().any(|e| matches!(
            e,
            StageEvent::StageAdvanced {
                from_stage_index: 0,
                to_stage_index: Some(1),
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            StageEvent::StageEntered { stage_name, .. } if stage_name == "second"
        )));
    }

    #[test]
    fn runner_ignores_artifacts_without_workflow_tag() {
        // A plain artifact with no workflow tag must not be picked up by
        // the runner — those belong to the existing kernel path.
        let mut store = ArtifactStore::new();
        let untagged = store.register(Kind::Code, "plain", "marvin");
        let eval = MockEvaluator::new();
        let workflows: HashMap<String, Workflow> = HashMap::new();
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        // No panic, no change.
        assert_eq!(store.get(&untagged).unwrap().state, ArtifactState::Draft);
    }

    #[test]
    fn missing_workflow_logs_but_does_not_panic() {
        // Simulates: active workflow deleted while an artifact is in-flight.
        let wf = make_workflow(
            "wf_gone",
            vec![make_stage("s", Some(ArtifactState::InProgress), vec![])],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "orphan");
        // Intentionally empty map.
        let workflows: HashMap<String, Workflow> = HashMap::new();
        let eval = MockEvaluator::new();
        let events = advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert!(events.is_empty());
        // Artifact is unchanged — runner refuses to act without the workflow.
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(0));
    }

    #[test]
    fn gate_passed_events_emit_only_when_stage_advances() {
        // A gate that passes while another is pending should NOT emit a
        // GatePassed event every tick — otherwise the stream spams
        // duplicate events (issue #80 copilot-review).
        let wf = make_workflow(
            "wf_noise",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![
                    GateSpec::AgentSession {
                        shaping_intent: serde_json::Value::Null,
                    },
                    GateSpec::ManualApproval {
                        signal_kind: "approve".into(),
                    },
                ],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "noisy");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        // agent_session passes, manual_approval pending.
        let mut eval = MockEvaluator::new();
        eval.set(id.as_str(), "agent_session", GateOutcome::Pass);

        // Three ticks with the same outcome — must not emit GatePassed
        // over and over.
        let mut total_gate_passed = 0;
        for _ in 0..3 {
            let events = advance_workflow_artifacts(&workflows, &mut store, &eval);
            total_gate_passed += events
                .iter()
                .filter(|e| matches!(e, StageEvent::GatePassed { .. }))
                .count();
        }
        assert_eq!(
            total_gate_passed, 0,
            "no GatePassed events should fire while another gate is pending"
        );

        // Once both pass, the runner emits a single batch.
        eval.set(id.as_str(), "manual_approval", GateOutcome::Pass);
        let events = advance_workflow_artifacts(&workflows, &mut store, &eval);
        let batch = events
            .iter()
            .filter(|e| matches!(e, StageEvent::GatePassed { .. }))
            .count();
        assert_eq!(batch, 2);
    }

    #[test]
    fn gate_failed_events_emit_only_on_park_state_change() {
        // Two ticks with the same failing gate: the second tick is a
        // no-op because the parked reason already matches.
        let wf = make_workflow(
            "wf_quiet_fail",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![GateSpec::ExternalCheck {
                    check_name: "ci".into(),
                }],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "quiet");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        eval.set(
            id.as_str(),
            "external_check",
            GateOutcome::Fail("red".into()),
        );

        let tick1 = advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(
            tick1
                .iter()
                .filter(|e| matches!(e, StageEvent::GateFailed { .. }))
                .count(),
            1
        );
        let tick2 = advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(
            tick2
                .iter()
                .filter(|e| matches!(e, StageEvent::GateFailed { .. }))
                .count(),
            0,
            "re-emitting the same failure each tick would spam the stream"
        );
    }

    #[test]
    fn out_of_bounds_stage_index_parks_artifact() {
        // Workflow is edited to drop stages while an artifact is at
        // stage_index=5. The runner must park it with a clear reason
        // and drop the stage tag so it stops being re-evaluated.
        let wf = make_workflow(
            "wf_shrunk",
            vec![make_stage("only", Some(ArtifactState::InProgress), vec![])],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "stranded");
        // Simulate the pre-shrink state by jamming stage_index past the end.
        store.get_mut(&id).unwrap().current_stage_index = Some(5);
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let eval = MockEvaluator::new();
        advance_workflow_artifacts(&workflows, &mut store, &eval);

        let art = store.get(&id).unwrap();
        assert_eq!(art.current_stage_index, None);
        assert!(art
            .workflow_parked_reason
            .as_deref()
            .unwrap()
            .contains("out of bounds"));
    }

    #[test]
    fn pending_tick_clears_stale_park_reason() {
        // Artifact parked by a prior failure; next tick the failure
        // clears but another gate is still pending. Parked reason
        // should drop so the dashboard reflects the current state,
        // not a stale failure (issue #80 copilot-review).
        let wf = make_workflow(
            "wf_recover_pending",
            vec![make_stage(
                "s0",
                Some(ArtifactState::InProgress),
                vec![
                    GateSpec::ExternalCheck {
                        check_name: "ci".into(),
                    },
                    GateSpec::ManualApproval {
                        signal_kind: "approve".into(),
                    },
                ],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "recover-pending");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        eval.set(
            id.as_str(),
            "external_check",
            GateOutcome::Fail("red".into()),
        );
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert!(store.get(&id).unwrap().workflow_parked_reason.is_some());

        // CI goes green; manual_approval still pending.
        eval.set(id.as_str(), "external_check", GateOutcome::Pass);
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        let art = store.get(&id).unwrap();
        // Still at stage 0 (manual_approval pending) but park reason cleared.
        assert_eq!(art.current_stage_index, Some(0));
        assert!(art.workflow_parked_reason.is_none());
    }

    #[test]
    fn on_stage_advanced_is_called() {
        // The runner invokes the evaluator's on_stage_advanced hook when
        // the artifact moves on, so per-stage scoped state can be dropped.
        use std::sync::Mutex;

        struct SpyEval {
            inner: MockEvaluator,
            advanced: Mutex<Vec<(String, u32)>>,
        }
        impl GateEvaluator for SpyEval {
            fn evaluate(
                &self,
                artifact: &Artifact,
                workflow: &Workflow,
                stage_index: u32,
                gate: &GateSpec,
            ) -> GateOutcome {
                self.inner.evaluate(artifact, workflow, stage_index, gate)
            }
            fn on_stage_advanced(&self, artifact_id: &ArtifactId, stage_index: u32) {
                self.advanced
                    .lock()
                    .unwrap()
                    .push((artifact_id.as_str().to_string(), stage_index));
            }
        }

        let wf = make_workflow(
            "wf_hook",
            vec![
                make_stage(
                    "s0",
                    Some(ArtifactState::InProgress),
                    vec![GateSpec::ManualApproval {
                        signal_kind: "a".into(),
                    }],
                ),
                make_stage("s1", Some(ArtifactState::UnderReview), vec![]),
            ],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "hook-test");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut inner = MockEvaluator::new();
        inner.set(id.as_str(), "manual_approval", GateOutcome::Pass);
        let eval = SpyEval {
            inner,
            advanced: Mutex::new(Vec::new()),
        };

        advance_workflow_artifacts(&workflows, &mut store, &eval);
        let calls = eval.advanced.lock().unwrap().clone();
        assert_eq!(calls, vec![(id.as_str().to_string(), 0)]);
    }

    #[test]
    fn manual_approval_only_resolves_on_matching_signal() {
        // A manual-approval gate is pending by default. The only way the
        // mock evaluator returns Pass is if we explicitly set it — which
        // mirrors production, where Pass requires a matching spine signal.
        let wf = make_workflow(
            "wf_manual",
            vec![make_stage(
                "merge",
                Some(ArtifactState::InProgress),
                vec![GateSpec::ManualApproval {
                    signal_kind: "pr_merged".into(),
                }],
            )],
        );
        let mut store = ArtifactStore::new();
        let id = enroll(&mut store, &wf, "manual-test");
        let mut workflows = HashMap::new();
        workflows.insert(wf.workflow_id.clone(), wf);

        let mut eval = MockEvaluator::new();
        // No outcome set — defaults to Pending.
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, Some(0));
        // Still at stage 0 — manual approval hasn't arrived.
        assert_eq!(store.get(&id).unwrap().state, ArtifactState::InProgress);

        // Explicit Pass — advances past end of chain.
        eval.set(id.as_str(), "manual_approval", GateOutcome::Pass);
        advance_workflow_artifacts(&workflows, &mut store, &eval);
        assert_eq!(store.get(&id).unwrap().current_stage_index, None);
    }
}
