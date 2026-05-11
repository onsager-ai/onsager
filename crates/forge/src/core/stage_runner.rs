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
///
/// Per spec #285 the per-gate `GatePassed` / `GateFailed` variants are
/// gone; the run timeline reconstructs gate outcomes from
/// `synodic.gate_verdict` and the stage advancement signal. Parking on
/// failure is still tracked on the artifact row's
/// `workflow_parked_reason`, just no longer mirrored as a spine event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageEvent {
    /// The artifact transitioned into a new stage.
    StageEntered {
        artifact_id: String,
        workflow_id: String,
        stage_index: u32,
        stage_name: String,
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

    for gate in &stage.gates {
        match evaluator.evaluate(&artifact_snapshot, workflow, stage_index, gate) {
            GateOutcome::Pass => {}
            GateOutcome::Fail(reason) => {
                gate_failures.push((gate.kind_tag().to_string(), reason));
            }
            GateOutcome::Pending => any_pending = true,
        }
    }

    if !gate_failures.is_empty() {
        // Park in UnderReview with the combined failure reason. Only
        // touch the row when the park reason actually changes so
        // repeated ticks over the same failing condition don't keep
        // bumping `updated_at`. Per spec #285 the per-gate
        // `stage.gate_failed` mirror event is gone; the parked reason
        // on the artifact row is the durable record.
        let parked_reason = gate_failures
            .iter()
            .map(|(k, r)| format!("{k}: {r}"))
            .collect::<Vec<_>>()
            .join("; ");
        let existing_reason = store
            .get(artifact_id)
            .and_then(|a| a.workflow_parked_reason.clone());
        if existing_reason.as_deref() != Some(parked_reason.as_str()) {
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

    // All gates passed — advance the artifact to the next stage. Per
    // spec #285 we no longer emit a per-gate `stage.gate_passed`;
    // `stage.advanced` (below) is the durable signal.
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
    if let Some(artifact) = store.get_mut(id)
        && artifact.state != target
        && artifact.state.can_transition_to(target)
    {
        artifact.state = target;
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
        if let Some(target) = first_stage.target_state
            && artifact.state != target
            && artifact.state.can_transition_to(target)
        {
            artifact.state = target;
        }
    }

    Some(StageEvent::StageEntered {
        artifact_id: artifact_id.as_str().to_string(),
        workflow_id: workflow.workflow_id.clone(),
        stage_index: 0,
        stage_name: first_stage.name.clone(),
    })
}

#[path = "stage_runner_tests.rs"]
#[cfg(test)]
mod tests;
