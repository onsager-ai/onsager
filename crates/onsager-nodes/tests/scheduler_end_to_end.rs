//! End-to-end: Spec Plan → compile → schedule.
//!
//! RUN-01 (#359) verification line:
//!
//! > End-to-end: Spec Plan → compile → schedule → all nodes execute
//! > → artifacts created in spine
//!
//! The "spine" here is the [`InMemoryPlanStore`] standing in for the
//! sqlx-backed adapter — the contract is identical (artifacts
//! materialize, keyed by `ArtifactId`). The real spine adapter slots
//! in via the [`PlanStore`] trait when MIG-* lands.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, NodeId, Provenance};
use onsager_nodes::{
    ExecutorRegistry, InMemoryPlanStore, PlanId, PlanStore, Scheduler, SpineClient, SpineError,
};
use onsager_substrate::compiler::compile;
use onsager_substrate::executor::NoOpExecutor as SubstrateNoOp;
use onsager_substrate::ids::{EdgeId, WorkflowId};
use onsager_substrate::library::WorkflowLibrary;
use onsager_substrate::spec_plan::{SpecDep, SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};

#[derive(Debug, Default)]
struct StubSpine;

#[async_trait]
impl SpineClient for StubSpine {
    async fn emit(&self, _: &str, _: serde_json::Value) -> Result<(), SpineError> {
        Ok(())
    }
    async fn read_artifact(&self, _: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
        Ok(None)
    }
}

/// Tiny test library — same shape as the one in
/// `onsager-substrate/src/compiler.rs` tests.
struct TestLibrary {
    by_id: std::collections::HashMap<WorkflowId, Workflow>,
    by_kind: std::collections::HashMap<String, WorkflowId>,
}

impl TestLibrary {
    fn new() -> Self {
        Self {
            by_id: Default::default(),
            by_kind: Default::default(),
        }
    }
    fn register(&mut self, kind: &str, w: Workflow) {
        let id = WorkflowId::generate();
        self.by_id.insert(id, w);
        self.by_kind.insert(kind.to_string(), id);
    }
}

impl WorkflowLibrary for TestLibrary {
    fn get(&self, id: WorkflowId) -> Option<&Workflow> {
        self.by_id.get(&id)
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        self.by_kind.get(kind).and_then(|id| self.by_id.get(id))
    }
}

/// `(entry) → [NoOp] → (exit)`
fn passthrough_workflow() -> Workflow {
    let entry = EdgeId::generate();
    let exit = EdgeId::generate();
    Workflow {
        nodes: vec![Node {
            id: NodeId::generate(),
            executor: Box::new(SubstrateNoOp),
            inputs: vec![EdgeRef::new(entry)],
            outputs: vec![EdgeRef::new(exit)],
        }],
        edges: vec![
            Edge {
                id: entry,
                artifact_id: ArtifactId::new("in"),
                requires_deterministic: false,
            },
            Edge {
                id: exit,
                artifact_id: ArtifactId::new("out"),
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
async fn spec_plan_compile_schedule_runs_all_nodes() {
    // Spec Plan: two passthrough specs, s1 → s2.
    let mut lib = TestLibrary::new();
    lib.register("passthrough", passthrough_workflow());

    let plan = SpecPlan {
        specs: vec![
            SpecRef {
                id: SpecId::new("s1"),
                kind: "passthrough".to_string(),
                inputs: Default::default(),
            },
            SpecRef {
                id: SpecId::new("s2"),
                kind: "passthrough".to_string(),
                inputs: Default::default(),
            },
        ],
        deps: vec![SpecDep {
            from: SpecId::new("s1"),
            to: SpecId::new("s2"),
        }],
    };

    let exec_plan = compile(&plan, &lib).expect("compile must succeed");
    assert_eq!(exec_plan.nodes.len(), 2);

    let scheduler = Scheduler::new(
        Arc::new(ExecutorRegistry::with_noop()),
        Arc::new(InMemoryPlanStore::new()) as Arc<dyn PlanStore>,
        Arc::new(StubSpine) as Arc<dyn SpineClient>,
    );
    let plan_id = PlanId::generate();
    scheduler
        .run(&plan_id, &exec_plan)
        .await
        .expect("schedule must succeed");

    // Every node terminal == Completed.
    let states = scheduler.store.node_states(&plan_id).await.unwrap();
    assert_eq!(states.len(), 2);
    for s in states.values() {
        assert_eq!(*s, onsager_nodes::NodeState::Completed);
    }
}
