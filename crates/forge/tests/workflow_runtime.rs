//! Integration tests for the workflow runtime (issue #80).
//!
//! Covers the full "issue → PR → CI → manual-merge" fixture workflow:
//! fires a trigger, asserts an artifact is registered, then walks it
//! through all four gate kinds by pushing the corresponding signals
//! into the [`SignalCache`] (the same entry point the real spine
//! listener uses).

use std::collections::HashMap;

use forge::core::artifact_store::ArtifactStore;
use forge::core::pipeline::{StiglabDispatcher, SynodicGate};
use forge::core::signal_cache::{Signal, SignalCache, SignalOutcome};
use forge::core::stage_runner::advance_workflow_artifacts;
use forge::core::trigger_subscriber::{register_artifact_from_trigger, TriggerFired};
use forge::core::workflow::{GateSpec, StageSpec, TriggerSpec, Workflow};
use forge::core::workflow_gates::{
    external_check_signal_kind, LiveGateEvaluator, AGENT_SESSION_SIGNAL,
};
use forge::core::workflow_signal_listener::classify_signal;
use onsager_artifact::ArtifactState;
use onsager_spine::factory_event::{FactoryEventKind, ShapingOutcome};
use onsager_spine::protocol::{GateRequest, GateVerdict, ShapingRequest, ShapingResult};

struct AllowSynodic;
impl SynodicGate for AllowSynodic {
    fn evaluate(&self, _req: &GateRequest) -> GateVerdict {
        GateVerdict::Allow
    }
}

struct NoopStiglab;
impl StiglabDispatcher for NoopStiglab {
    fn dispatch(&self, req: &ShapingRequest) -> ShapingResult {
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

fn fixture_workflow() -> Workflow {
    Workflow {
        workflow_id: "wf_issue_to_pr".into(),
        name: "issue-to-pr".into(),
        trigger: TriggerSpec::GithubIssueWebhook {
            repo: "onsager-ai/onsager".into(),
            label: "ai-implementable".into(),
        },
        stages: vec![
            StageSpec {
                name: "implement".into(),
                target_state: Some(ArtifactState::InProgress),
                gates: vec![GateSpec::AgentSession {
                    shaping_intent: serde_json::json!({"role": "coder"}),
                }],
                params: serde_json::Value::Null,
            },
            StageSpec {
                name: "governance".into(),
                target_state: Some(ArtifactState::UnderReview),
                gates: vec![GateSpec::Governance { gate_point: None }],
                params: serde_json::Value::Null,
            },
            StageSpec {
                name: "ci".into(),
                target_state: None,
                gates: vec![GateSpec::ExternalCheck {
                    check_name: "ci/test".into(),
                }],
                params: serde_json::Value::Null,
            },
            StageSpec {
                name: "merge".into(),
                target_state: Some(ArtifactState::Released),
                gates: vec![GateSpec::ManualApproval {
                    signal_kind: "pr_merged".into(),
                }],
                params: serde_json::Value::Null,
            },
        ],
        active: true,
        preset_id: Some("github_issue_to_pr".into()),
        workspace_install_ref: None,
        created_by: None,
    }
}

fn trigger_for(workflow: &Workflow) -> TriggerFired {
    TriggerFired {
        event_id: 1,
        workflow_id: workflow.workflow_id.clone(),
        trigger_kind: "github_issue_webhook".into(),
        payload: serde_json::json!({
            "title": "Fix the thing",
            "owner": "marvin",
            "issue_number": 42,
        }),
    }
}

#[test]
fn trigger_fired_registers_artifact_tagged_with_workflow() {
    let wf = fixture_workflow();
    let trigger = trigger_for(&wf);
    let mut store = ArtifactStore::new();
    let (artifact, _event) =
        register_artifact_from_trigger(&mut store, &wf, &trigger).expect("registered");
    assert_eq!(artifact.workflow_id.as_deref(), Some("wf_issue_to_pr"));
    assert_eq!(artifact.current_stage_index, Some(0));
    assert_eq!(artifact.kind.to_string(), "github-issue");
}

#[test]
fn artifact_walks_issue_to_pr_to_ci_to_merge() {
    // End-to-end happy path. Each gate kind resolves in turn via the
    // signal cache (the real spine listener's entry point).
    let wf = fixture_workflow();
    let trigger = trigger_for(&wf);
    let mut store = ArtifactStore::new();
    let (artifact, _) = register_artifact_from_trigger(&mut store, &wf, &trigger).unwrap();
    let id = artifact.artifact_id.clone();

    let signals = SignalCache::new();
    let evaluator = LiveGateEvaluator::new(signals.clone(), NoopStiglab, AllowSynodic);
    let mut workflows = HashMap::new();
    workflows.insert(wf.workflow_id.clone(), wf);

    // Tick 1: agent-session dispatches, stays pending.
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(0));

    // Agent session completes → signal arrives → stage 0 passes, advances to stage 1.
    signals.push(
        id.as_str(),
        Signal {
            kind: AGENT_SESSION_SIGNAL.into(),
            outcome: SignalOutcome::Success,
        },
    );
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(1));
    // Stage 1 is Governance (Allow) — advances to stage 2 same tick.
    // Because AllowSynodic returns Allow synchronously, the next tick covers it.
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(2));

    // Stage 2 is external-check on ci/test. Push a failing signal first.
    signals.push(
        id.as_str(),
        Signal {
            kind: external_check_signal_kind("ci/test"),
            outcome: SignalOutcome::Failure("red".into()),
        },
    );
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    // Parks artifact in UnderReview (already is), stays at stage 2.
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(2));
    assert_eq!(store.get(&id).unwrap().state, ArtifactState::UnderReview);
    assert!(store.get(&id).unwrap().workflow_parked_reason.is_some());

    // Rerun goes green → advances to stage 3.
    signals.push(
        id.as_str(),
        Signal {
            kind: external_check_signal_kind("ci/test"),
            outcome: SignalOutcome::Success,
        },
    );
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(3));
    assert!(store.get(&id).unwrap().workflow_parked_reason.is_none());

    // Stage 3: manual-approval waits for pr_merged signal.
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(3));

    // Merge webhook → artifact completes workflow.
    signals.push(
        id.as_str(),
        Signal {
            kind: "pr_merged".into(),
            outcome: SignalOutcome::Success,
        },
    );
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    let final_artifact = store.get(&id).unwrap();
    assert_eq!(final_artifact.current_stage_index, None);
    assert_eq!(final_artifact.state, ArtifactState::Released);
}

#[test]
fn signal_listener_classifier_feeds_stage_runner() {
    // Verifies the listener → signal cache → gate evaluator handoff end
    // to end. The classifier is the pure half of the listener; pushing
    // its output into the cache is what the real listener does on
    // pg_notify events.
    let wf = fixture_workflow();
    let trigger = trigger_for(&wf);
    let mut store = ArtifactStore::new();
    let (artifact, _) = register_artifact_from_trigger(&mut store, &wf, &trigger).unwrap();
    let id = artifact.artifact_id.clone();

    let signals = SignalCache::new();
    let evaluator = LiveGateEvaluator::new(signals.clone(), NoopStiglab, AllowSynodic);
    let mut workflows = HashMap::new();
    workflows.insert(wf.workflow_id.clone(), wf);

    // Simulate a session_completed spine event.
    let event = FactoryEventKind::StiglabSessionCompleted {
        session_id: "s".into(),
        request_id: "r".into(),
        duration_ms: 1,
        artifact_id: Some(id.as_str().to_string()),
        token_usage: None,
        branch: None,
        pr_number: None,
    };
    let (aid, sig) = classify_signal(&event).expect("classified");
    signals.push(&aid, sig);

    // First tick: dispatches; pending.
    // Second tick: gate now sees the signal and passes → advances.
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert!(store.get(&id).unwrap().current_stage_index.unwrap() >= 1);
}

#[test]
fn deactivating_workflow_stops_new_triggers_but_allows_inflight_to_finish() {
    let wf = fixture_workflow();
    let trigger = trigger_for(&wf);
    let mut store = ArtifactStore::new();
    let (artifact, _) = register_artifact_from_trigger(&mut store, &wf, &trigger).unwrap();
    let id = artifact.artifact_id.clone();

    // Deactivate the workflow mid-run (keep the in-memory record; this
    // matches how `PATCH /api/workflows/:id` would set active=false).
    let mut deactivated = wf.clone();
    deactivated.active = false;

    // Firing the trigger again must not register a new artifact.
    let dropped = register_artifact_from_trigger(&mut store, &deactivated, &trigger);
    assert!(dropped.is_none());

    // But the in-flight artifact keeps progressing through the runner.
    let signals = SignalCache::new();
    let evaluator = LiveGateEvaluator::new(signals.clone(), NoopStiglab, AllowSynodic);
    let mut workflows = HashMap::new();
    workflows.insert(deactivated.workflow_id.clone(), deactivated);

    signals.push(
        id.as_str(),
        Signal {
            kind: AGENT_SESSION_SIGNAL.into(),
            outcome: SignalOutcome::Success,
        },
    );
    advance_workflow_artifacts(&workflows, &mut store, &evaluator);
    assert_eq!(store.get(&id).unwrap().current_stage_index, Some(1));
}
