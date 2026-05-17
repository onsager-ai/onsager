//! Integration test: TriggerFired payload → bridge → scheduler →
//! captured spine emits.
//!
//! Verifies the v1 contract — payload carries a `spec_kind`, the
//! library returns a Workflow, the bridge compiles and runs it, the
//! scheduler emits node lifecycle events to the spine.
//!
//! No Postgres dependency: the [`SpineClient`] is a capturing mock
//! and the workflow is provided via the in-process
//! `PreloadedWorkflow` adapter. The end-to-end with a real spine
//! lives in `tests/spine_listener.rs` when the host's Postgres
//! becomes addressable in CI; the bridge contract proven here is
//! the same shape that path would execute.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance};
use onsager_nodes::{
    Executor, ExecutorContext, ExecutorError, ExecutorOutputs, ExecutorRegistry, SpineClient,
    SpineError,
};
use onsager_scheduler::{TriggerBridge, bridge::PreloadedWorkflow, bridge::WorkflowMeta};
use onsager_substrate::ids::EdgeId;
use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};

#[derive(Debug, Default)]
struct CapturingSpine {
    emitted: Mutex<Vec<(String, serde_json::Value)>>,
}

#[async_trait]
impl SpineClient for CapturingSpine {
    async fn emit(&self, kind: &str, payload: serde_json::Value) -> Result<(), SpineError> {
        self.emitted
            .lock()
            .unwrap()
            .push((kind.to_string(), payload));
        Ok(())
    }
    async fn read_artifact(&self, _: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
        Ok(None)
    }
}

#[derive(Debug)]
struct EmitOne;

#[async_trait]
impl Executor for EmitOne {
    fn executor_kind(&self) -> &'static str {
        "noop"
    }
    fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
        Provenance::external_deterministic()
    }
    async fn execute(&self, _: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let art = Artifact::new(Kind::Document, "round-trip", "marvin", "test", vec![]);
        let id = art.artifact_id.clone();
        Ok(ExecutorOutputs::single(id, art))
    }
}

fn build_workflow() -> Workflow {
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
                artifact_id: ArtifactId::new("e2e-in"),
                requires_deterministic: false,
            },
            Edge {
                id: exit,
                artifact_id: ArtifactId::new("e2e-out"),
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

#[tokio::test]
async fn trigger_fired_drives_scheduler_and_emits_node_events() {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(EmitOne));
    let spine = Arc::new(CapturingSpine::default());
    let bridge = TriggerBridge::new(
        Arc::new(registry),
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    );

    // Simulate a manual fire: `onsager trigger fire wf-e2e --manual run --payload '{"spec_kind":"e2e-kind"}'`
    let payload = serde_json::json!({
        "spec_kind": "e2e-kind",
        "trigger_kind": "manual",
        "actor": "test-runner",
    });
    let lookup = PreloadedWorkflow {
        kind: "e2e-kind".to_string(),
        workflow: Some(build_workflow()),
    };
    let plan_id = bridge
        .handle_payload("wf-e2e", &payload, &WorkflowMeta::default(), lookup)
        .await
        .expect("bridge runs the plan to completion");

    let emitted = spine.emitted.lock().unwrap().clone();
    let kinds: Vec<&str> = emitted.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        kinds.contains(&"node.started"),
        "expected node.started, got {kinds:?}",
    );
    assert!(
        kinds.contains(&"node.completed"),
        "expected node.completed, got {kinds:?}",
    );

    // Every spine event was tagged with our PlanId — proves the
    // trigger handle threaded the same plan_id through every emit.
    for (kind, payload) in &emitted {
        let got = payload
            .get("plan_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            got,
            plan_id.as_str(),
            "{kind} payload plan_id should equal the bridge's PlanId",
        );
    }
}
