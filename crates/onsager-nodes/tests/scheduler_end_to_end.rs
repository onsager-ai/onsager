//! End-to-end: Spec Plan → compile → schedule → artifacts persisted.
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
use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance};
use onsager_nodes::{
    Executor, ExecutorContext, ExecutorError, ExecutorOutputs, ExecutorRegistry, InMemoryPlanStore,
    NodeState, PlanId, PlanStore, Scheduler, SpineClient, SpineError,
};
use onsager_substrate::compiler::compile;
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

/// Producer executor — emits exactly one declared artifact so the
/// e2e test can prove that `PlanStore::get_artifact` reads back what
/// the executor produced.
///
/// Registers under the `noop` kind so it overrides the default
/// `NoOpExecutor`, which is what the substrate-side `passthrough_workflow`
/// nodes carry.
#[derive(Debug)]
struct ProducerExecutor {
    name: &'static str,
}

#[async_trait]
impl Executor for ProducerExecutor {
    fn executor_kind(&self) -> &'static str {
        "noop"
    }
    fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
        Provenance::external_deterministic()
    }
    async fn execute(&self, _: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let art = Artifact::new(Kind::Document, self.name, "marvin", "test", vec![]);
        // The scheduler reconciles the artifact's id with the edge's
        // ArtifactId at persist time, so we don't have to thread it
        // through here.
        let id = art.artifact_id.clone();
        Ok(ExecutorOutputs::single(id, art))
    }
}

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

/// `(entry) → [NoOp] → (exit)` — substrate-side workflow whose nodes
/// the e2e test will dispatch through a `ProducerExecutor` runtime.
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
async fn spec_plan_compile_schedule_materializes_artifacts() {
    // Spec Plan: two passthrough specs, s1 → s2. Each instantiates
    // one node, namespaced by spec id; the compiler rewires s2's
    // entry to s1's exit edge.
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

    // Wire a producer executor under the `noop` kind so each node
    // actually emits an artifact.
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(ProducerExecutor {
        name: "passthrough-output",
    }));
    let store = Arc::new(InMemoryPlanStore::new());
    let scheduler = Scheduler::new(
        Arc::new(registry),
        Arc::clone(&store) as Arc<dyn PlanStore>,
        Arc::new(StubSpine) as Arc<dyn SpineClient>,
    );
    let plan_id = PlanId::generate();
    scheduler
        .run(&plan_id, &exec_plan)
        .await
        .expect("schedule must succeed");

    // Every node terminated Completed.
    let states = store.node_states(&plan_id).await.unwrap();
    assert_eq!(states.len(), 2);
    for s in states.values() {
        assert_eq!(*s, NodeState::Completed);
    }

    // RUN-01 verification: artifacts created in spine. Each node's
    // declared output edge carries a namespaced ArtifactId; the
    // scheduler persisted one artifact under each, reconciled to
    // that edge's id.
    let mut output_edges: Vec<&ArtifactId> = exec_plan
        .edges
        .iter()
        .filter(|e| {
            exec_plan
                .nodes
                .iter()
                .any(|n| n.outputs.iter().any(|o| o.edge_id == e.id))
        })
        .map(|e| &e.artifact_id)
        .collect();
    output_edges.sort_by_key(|id| id.as_str().to_string());
    assert_eq!(output_edges.len(), 2);

    for art_id in &output_edges {
        let materialized = store
            .get_artifact(&plan_id, art_id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("expected artifact materialized at {art_id}"));
        // The scheduler reconciles the persisted artifact's own id
        // with the edge's id — body and key agree.
        assert_eq!(
            &materialized.artifact_id, *art_id,
            "persisted artifact's id must equal the edge's ArtifactId",
        );
    }
}
