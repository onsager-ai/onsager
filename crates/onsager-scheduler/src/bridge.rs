//! [`TriggerBridge`] — turns a `trigger.fired` payload into one
//! [`ExecutionPlan`] run.
//!
//! ## v1 contract
//!
//! The bridge resolves the workflow `spec_kind` from the trigger
//! event in two steps:
//!
//! 1. The trigger payload may carry an explicit `"spec_kind": "..."`
//!    field. When present, this wins. Manual fires
//!    (`onsager trigger fire ... --payload '{"spec_kind": "..."}'`)
//!    use this path today.
//! 2. Otherwise, the bridge falls back to the workflow row's
//!    `preset_id` (per `crates/onsager-spine/migrations/006_workflows.sql`).
//!    Workflows created via a portal preset (`github-issue-to-pr`)
//!    carry one; webhook-fired workflows that don't pin a preset
//!    fall through to a structured warning and the event is dropped.
//!
//! The resolved `spec_kind` is looked up in the Workflow Library
//! (SUB-04, #351). A missing kind is also a structured warning —
//! the v1 contract is "best-effort dispatch": one drift between the
//! trigger emitter and the library is logged loudly, not silently
//! retried. Bringing the bridge to first-class status (workflow row
//! gains a `spec_kind` column, manifest-driven dispatch) is a
//! follow-up rather than part of RUN-03.
//!
//! ## Single-spec SpecPlan
//!
//! v1 builds a one-element [`SpecPlan`] per trigger: the resolved
//! `spec_kind` is the spec's `kind`, and its id is derived from the
//! workflow id. Multi-spec orchestration (Refract producing a
//! many-spec plan) is a layer above this bridge — when it lands, it
//! will hand the bridge a pre-built [`SpecPlan`] instead.

use std::collections::HashMap;
use std::sync::Arc;

use onsager_nodes::{InMemoryPlanStore, PlanId, PlanStore, Scheduler, SchedulerError, SpineClient};
use onsager_substrate::compiler::{CompileError, compile};
use onsager_substrate::ids::WorkflowId;
use onsager_substrate::library::WorkflowLibrary as WorkflowLookup;
use onsager_substrate::spec_plan::{SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::Workflow;
use serde_json::Value;
use thiserror::Error;

/// Outcome of running [`TriggerBridge::handle_payload`].
#[derive(Debug, Error)]
pub enum TriggerBridgeError {
    /// The trigger payload did not name a `spec_kind` and no fallback
    /// resolved one. The event is dropped with a structured log.
    #[error(
        "trigger payload missing spec_kind for workflow `{workflow_id}` (no payload field, no preset fallback)"
    )]
    UnresolvedSpecKind { workflow_id: String },

    /// No workflow registered under the resolved kind.
    #[error("no workflow registered for spec_kind `{kind}` (workflow `{workflow_id}`)")]
    MissingKind { workflow_id: String, kind: String },

    /// Compile-step failure — Spec Plan invalid, kernel invariant
    /// violated, etc.
    #[error("compile failed for workflow `{workflow_id}` (kind `{kind}`): {source}")]
    Compile {
        workflow_id: String,
        kind: String,
        #[source]
        source: CompileError,
    },

    /// Scheduler-step failure — a node failed, the plan got stuck.
    #[error("scheduler failed for workflow `{workflow_id}` (kind `{kind}`): {source}")]
    Scheduler {
        workflow_id: String,
        kind: String,
        #[source]
        source: SchedulerError,
    },
}

/// Source of a workflow's `spec_kind` for an incoming trigger. The
/// service layer fetches one of these per `trigger.fired` workflow id;
/// the bridge consumes it without touching the DB itself so tests
/// drive the same control-flow as production.
#[derive(Debug, Clone, Default)]
pub struct WorkflowMeta {
    /// `preset_id` from the workflow row, when present. Used as the
    /// fallback `spec_kind` when the trigger payload omits one.
    pub preset_id: Option<String>,
}

/// Stateless bridge: trigger payload + workflow library → one
/// executed plan.
///
/// Holds the scheduler-side wiring (registry, store, spine) once at
/// construction time; each call to [`Self::handle_payload`] builds a
/// fresh [`PlanId`] and runs the resolved plan through
/// [`Scheduler::run`].
#[derive(Clone)]
pub struct TriggerBridge {
    registry: Arc<onsager_nodes::ExecutorRegistry>,
    spine: Arc<dyn SpineClient>,
}

impl std::fmt::Debug for TriggerBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerBridge").finish_non_exhaustive()
    }
}

impl TriggerBridge {
    pub fn new(
        registry: Arc<onsager_nodes::ExecutorRegistry>,
        spine: Arc<dyn SpineClient>,
    ) -> Self {
        Self { registry, spine }
    }

    /// Run a single trigger.fired payload to completion.
    ///
    /// `workflow_id` is the spine workflow id from the
    /// `FactoryEventKind::TriggerFired` envelope. `payload` is the
    /// raw JSON the trigger emitter attached. `meta` carries
    /// optional fallback data the service layer fetched from the
    /// `workflows` row. `library` resolves the chosen `spec_kind`
    /// to a [`Workflow`] template.
    pub async fn handle_payload(
        &self,
        workflow_id: &str,
        payload: &Value,
        meta: &WorkflowMeta,
        library: impl WorkflowLookupOwned,
    ) -> Result<PlanId, TriggerBridgeError> {
        let kind = resolve_spec_kind(payload, meta).ok_or_else(|| {
            TriggerBridgeError::UnresolvedSpecKind {
                workflow_id: workflow_id.to_string(),
            }
        })?;

        let workflow =
            library
                .by_kind_owned(&kind)
                .ok_or_else(|| TriggerBridgeError::MissingKind {
                    workflow_id: workflow_id.to_string(),
                    kind: kind.clone(),
                })?;
        let spec_plan = SpecPlan {
            specs: vec![SpecRef {
                id: SpecId::new(format!("workflow:{workflow_id}")),
                kind: kind.clone(),
                inputs: Default::default(),
            }],
            deps: vec![],
        };
        let single = SingleWorkflowLookup::new(kind.clone(), workflow);
        let exec_plan =
            compile(&spec_plan, &single).map_err(|source| TriggerBridgeError::Compile {
                workflow_id: workflow_id.to_string(),
                kind: kind.clone(),
                source,
            })?;

        let store: Arc<dyn PlanStore> = Arc::new(InMemoryPlanStore::new());
        let scheduler = Scheduler::new(Arc::clone(&self.registry), store, Arc::clone(&self.spine));
        let plan_id = PlanId::generate();
        scheduler
            .run(&plan_id, &exec_plan)
            .await
            .map_err(|source| TriggerBridgeError::Scheduler {
                workflow_id: workflow_id.to_string(),
                kind,
                source,
            })?;
        Ok(plan_id)
    }
}

/// Lookup surface the bridge needs: take a kind, give back an owned
/// [`Workflow`] value (the persisted [`WorkflowLibrary`] returns by
/// value because it deserializes per-call from JSONB).
pub trait WorkflowLookupOwned {
    fn by_kind_owned(&self, kind: &str) -> Option<Workflow>;
}

/// Adapter exposing a single owned [`Workflow`] under one `spec_kind`
/// to the Plan Compiler's reference-borrowing [`WorkflowLookup`]
/// trait. Built fresh per trigger so the borrow stays scoped.
struct SingleWorkflowLookup {
    kind: String,
    workflow: Workflow,
    subworkflows: HashMap<WorkflowId, Workflow>,
}

impl SingleWorkflowLookup {
    fn new(kind: String, workflow: Workflow) -> Self {
        Self {
            kind,
            workflow,
            subworkflows: HashMap::new(),
        }
    }
}

impl WorkflowLookup for SingleWorkflowLookup {
    fn get(&self, id: WorkflowId) -> Option<&Workflow> {
        self.subworkflows.get(&id)
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        if kind == self.kind {
            Some(&self.workflow)
        } else {
            None
        }
    }
}

/// Service-layer helper: same resolution logic as the bridge uses
/// internally, exposed so the listener can prefetch the workflow
/// before calling [`TriggerBridge::handle_payload`]. Returning the
/// chosen kind here (instead of re-running the logic inside the
/// bridge) keeps the v1 resolution rule single-sourced.
pub fn resolve_spec_kind_for_logging(payload: &Value, meta: &WorkflowMeta) -> Option<String> {
    resolve_spec_kind(payload, meta)
}

/// Pick a `spec_kind` from the trigger payload, falling back to the
/// workflow row's `preset_id`. Returns `None` when neither resolves —
/// the caller surfaces that as [`TriggerBridgeError::UnresolvedSpecKind`].
fn resolve_spec_kind(payload: &Value, meta: &WorkflowMeta) -> Option<String> {
    if let Some(s) = payload.get("spec_kind").and_then(Value::as_str)
        && !s.is_empty()
    {
        return Some(s.to_string());
    }
    meta.preset_id.clone().filter(|s| !s.is_empty())
}

// Convenience for tests / production: implement the lookup-by-value
// trait directly on the persisted struct.
impl WorkflowLookupOwned for onsager_substrate::workflow_library::WorkflowLibrary {
    fn by_kind_owned(&self, _kind: &str) -> Option<Workflow> {
        // The persisted library is async; the sync surface here is
        // for the unit tests and the in-process bridge. The service
        // layer fetches the workflow via the async `lookup` first
        // and passes a [`PreloadedWorkflow`] adapter (below) to the
        // bridge — never calling this method.
        None
    }
}

/// Adapter the service layer uses: it fetches the workflow once via
/// the async [`onsager_substrate::workflow_library::WorkflowLibrary::lookup`]
/// and hands the result to the bridge through this sync wrapper.
pub struct PreloadedWorkflow {
    pub kind: String,
    pub workflow: Option<Workflow>,
}

impl WorkflowLookupOwned for PreloadedWorkflow {
    fn by_kind_owned(&self, kind: &str) -> Option<Workflow> {
        if kind == self.kind {
            // The workflow is moved into the lookup once; subsequent
            // calls would have to clone. The bridge calls once per
            // trigger so a `take` shape would also work, but using
            // serde round-trip here keeps the method `&self`.
            self.workflow.as_ref().and_then(|w| {
                serde_json::to_value(w)
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use onsager_artifact::{Artifact, ArtifactId, NodeId, Provenance};
    use onsager_nodes::{Executor, ExecutorContext, ExecutorError, ExecutorOutputs, SpineError};
    use onsager_substrate::ids::EdgeId;
    use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec};
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingSpine {
        emitted: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SpineClient for CapturingSpine {
        async fn emit(&self, kind: &str, _: serde_json::Value) -> Result<(), SpineError> {
            self.emitted.lock().unwrap().push(kind.to_string());
            Ok(())
        }
        async fn read_artifact(&self, _: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
            Ok(None)
        }
    }

    /// Echo executor — produces one declared artifact per call so the
    /// scheduler reports a node.completed with output_artifact_ids.
    #[derive(Debug)]
    struct EchoExecutor;

    #[async_trait]
    impl Executor for EchoExecutor {
        fn executor_kind(&self) -> &'static str {
            "noop"
        }
        fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
            Provenance::external_deterministic()
        }
        async fn execute(&self, _: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
            let art = Artifact::new(
                onsager_artifact::Kind::Document,
                "trigger-output",
                "marvin",
                "scheduler",
                vec![],
            );
            let id = art.artifact_id.clone();
            Ok(ExecutorOutputs::single(id, art))
        }
    }

    fn passthrough_workflow() -> Workflow {
        let entry = EdgeId::generate();
        let exit = EdgeId::generate();
        Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(onsager_substrate::executor::NoOpExecutor),
                inputs: vec![EdgeRef::new(entry)],
                outputs: vec![EdgeRef::new(exit)],
            }],
            edges: vec![
                Edge {
                    id: entry,
                    artifact_id: ArtifactId::new("trigger-in"),
                    requires_deterministic: false,
                },
                Edge {
                    id: exit,
                    artifact_id: ArtifactId::new("trigger-out"),
                    requires_deterministic: false,
                },
            ],
            entry_specs: vec![EntrySpec { edge_id: entry }],
            output_specs: vec![OutputSpec {
                edge_id: exit,
                provenance: Provenance::external_deterministic(),
            }],
        }
    }

    fn build_bridge() -> (TriggerBridge, Arc<CapturingSpine>) {
        let mut registry = onsager_nodes::ExecutorRegistry::new();
        registry.register(Arc::new(EchoExecutor));
        let spine = Arc::new(CapturingSpine::default());
        let bridge = TriggerBridge::new(
            Arc::new(registry),
            Arc::clone(&spine) as Arc<dyn SpineClient>,
        );
        (bridge, spine)
    }

    #[tokio::test]
    async fn payload_spec_kind_drives_run() {
        let (bridge, spine) = build_bridge();
        let payload = serde_json::json!({ "spec_kind": "echo" });
        let lookup = PreloadedWorkflow {
            kind: "echo".to_string(),
            workflow: Some(passthrough_workflow()),
        };
        bridge
            .handle_payload("wf-1", &payload, &WorkflowMeta::default(), lookup)
            .await
            .expect("happy path runs the scheduler to completion");

        let emitted = spine.emitted.lock().unwrap().clone();
        assert!(
            emitted.iter().any(|k| k == "node.started"),
            "expected node.started emit, got {emitted:?}",
        );
        assert!(
            emitted.iter().any(|k| k == "node.completed"),
            "expected node.completed emit, got {emitted:?}",
        );
    }

    #[tokio::test]
    async fn preset_id_falls_back_when_payload_omits_kind() {
        let (bridge, _) = build_bridge();
        let payload = serde_json::json!({});
        let meta = WorkflowMeta {
            preset_id: Some("echo".to_string()),
        };
        let lookup = PreloadedWorkflow {
            kind: "echo".to_string(),
            workflow: Some(passthrough_workflow()),
        };
        bridge
            .handle_payload("wf-2", &payload, &meta, lookup)
            .await
            .expect("preset fallback runs");
    }

    #[tokio::test]
    async fn unresolved_spec_kind_surfaces_as_error() {
        let (bridge, _) = build_bridge();
        let payload = serde_json::json!({});
        let lookup = PreloadedWorkflow {
            kind: "never-asked".to_string(),
            workflow: Some(passthrough_workflow()),
        };
        let err = bridge
            .handle_payload("wf-3", &payload, &WorkflowMeta::default(), lookup)
            .await
            .unwrap_err();
        assert!(matches!(err, TriggerBridgeError::UnresolvedSpecKind { .. }));
    }

    #[tokio::test]
    async fn missing_kind_surfaces_as_error() {
        let (bridge, _) = build_bridge();
        let payload = serde_json::json!({ "spec_kind": "echo" });
        let lookup = PreloadedWorkflow {
            kind: "different-kind".to_string(),
            workflow: Some(passthrough_workflow()),
        };
        let err = bridge
            .handle_payload("wf-4", &payload, &WorkflowMeta::default(), lookup)
            .await
            .unwrap_err();
        assert!(
            matches!(err, TriggerBridgeError::MissingKind { ref kind, .. } if kind == "echo"),
            "expected MissingKind(echo), got {err:?}",
        );
    }
}
