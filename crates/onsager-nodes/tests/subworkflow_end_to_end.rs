//! End-to-end: outer workflow → SubWorkflow node → inner workflow.
//!
//! EXE-05 (#357) verification lines:
//!
//! > - Outer workflow with SubWorkflow node referencing an inner
//! >   workflow (registered in WorkflowLibrary) runs end-to-end
//! > - Inner workflow's Uncertain output propagates to outer
//! >   workflow's Uncertain output
//!
//! The outer plan is one node — a SubWorkflow node pointing at the
//! inner workflow. When the substrate scheduler reaches the outer
//! node, it dispatches through the [`SubWorkflowExecutor`] runtime,
//! which in turn runs the inner workflow via the
//! [`SchedulerSubWorkflowRunner`]. The inner workflow's only node
//! emits an `Uncertain { source: Agent }` artifact; that artifact
//! flows through to the outer plan store with its provenance intact.
//!
//! The inner Scheduler and the outer Scheduler use *separate*
//! [`ExecutorRegistry`] instances. The inner registry holds the
//! runtime executors the inner workflow's nodes dispatch to; the
//! outer registry holds the [`SubWorkflowExecutor`] runtime. This
//! split avoids the Arc cycle that "share the same registry"
//! requires; recursive nesting (a SubWorkflow inside a SubWorkflow)
//! is the only case that would need an additional level of wiring
//! and is out of EXE-05's scope.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance, SourceTag};
use onsager_nodes::{
    Executor, ExecutorContext, ExecutorError, ExecutorOutputs, ExecutorRegistry, InMemoryPlanStore,
    NodeState, PlanId, PlanStore, Scheduler, SchedulerSubWorkflowRunner, SpineClient, SpineError,
    SubWorkflowExecutor,
};
use onsager_substrate::compiler::ExecutionPlan;
use onsager_substrate::ids::{EdgeId, WorkflowId};
use onsager_substrate::library::WorkflowLibrary;
use onsager_substrate::workflow::{Edge, EdgeRef, Node, OutputSpec, Workflow};

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

/// Inner-side executor that emits one `Uncertain { source: Agent }`
/// artifact per call. Registers under the `noop` kind so the inner
/// workflow's substrate `NoOpExecutor` nodes dispatch to it.
#[derive(Debug)]
struct UncertainEmittingExecutor;

#[async_trait]
impl Executor for UncertainEmittingExecutor {
    fn executor_kind(&self) -> &'static str {
        "noop"
    }
    fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
        Provenance::Uncertain {
            source: SourceTag::Agent,
        }
    }
    async fn execute(&self, _: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        let mut art = Artifact::new(Kind::Code, "inner-uncertain", "marvin", "test", vec![]);
        art.provenance = Provenance::Uncertain {
            source: SourceTag::Agent,
        };
        let id = art.artifact_id.clone();
        Ok(ExecutorOutputs::single(id, art))
    }
}

#[derive(Default)]
struct InMemoryLibrary {
    by_id: HashMap<WorkflowId, Workflow>,
}

impl WorkflowLibrary for InMemoryLibrary {
    fn get(&self, id: WorkflowId) -> Option<&Workflow> {
        self.by_id.get(&id)
    }
}

/// Inner workflow: one node emitting an Uncertain artifact on its
/// exit edge.
fn inner_workflow() -> Workflow {
    let exit_edge = EdgeId::generate();
    Workflow {
        nodes: vec![Node {
            id: NodeId::generate(),
            executor: Box::new(onsager_substrate::executor::NoOpExecutor),
            inputs: vec![],
            outputs: vec![EdgeRef::new(exit_edge)],
        }],
        edges: vec![Edge {
            id: exit_edge,
            artifact_id: ArtifactId::new("inner-exit"),
            requires_deterministic: false,
        }],
        entry_specs: vec![],
        output_specs: vec![OutputSpec {
            edge_id: exit_edge,
            provenance: Provenance::Uncertain {
                source: SourceTag::Agent,
            },
        }],
    }
}

#[tokio::test]
async fn outer_subworkflow_node_propagates_inner_uncertain_to_outer_exit() {
    // ----- Library + inner workflow -----
    let inner_id = WorkflowId::generate();
    let mut library = InMemoryLibrary::default();
    library.by_id.insert(inner_id, inner_workflow());
    let library: Arc<dyn WorkflowLibrary + Send + Sync> = Arc::new(library);

    // ----- Inner registry: holds the runtime executors the inner
    // workflow's nodes dispatch to. Owned by the SubWorkflow runner.
    let mut inner_registry = ExecutorRegistry::new();
    inner_registry.register(Arc::new(UncertainEmittingExecutor));
    let inner_registry = Arc::new(inner_registry);

    // ----- Shared store + spine (inner and outer share both; the
    // PlanStore key is `(plan_id, artifact_id)` so sub-runs and the
    // outer run live in distinct partitions).
    let store: Arc<dyn PlanStore> = Arc::new(InMemoryPlanStore::new());
    let spine: Arc<dyn SpineClient> = Arc::new(StubSpine);

    let runner = Arc::new(SchedulerSubWorkflowRunner::new(
        Arc::clone(&library),
        inner_registry,
        Arc::clone(&store),
        Arc::clone(&spine),
    ));

    // ----- Outer registry: holds the SubWorkflowExecutor runtime
    // wired with the runner above. The registered SubWorkflowExecutor
    // carries a placeholder workflow_ref — the scheduler reads the
    // per-node ref off the substrate executor at dispatch time and
    // threads it through `ExecutorContext::subworkflow_ref`.
    let mut outer_registry = ExecutorRegistry::new();
    outer_registry.register(Arc::new(
        SubWorkflowExecutor::new(WorkflowId::generate()).with_runner(runner),
    ));
    let outer_registry = Arc::new(outer_registry);

    // ----- Outer plan: a single SubWorkflow node pointing at the
    // inner workflow. The substrate executor's `subworkflow_ref()`
    // returns `Some(inner_id)`; the scheduler routes it to the
    // outer registry's SubWorkflow runtime.
    let outer_exit_edge = EdgeId::generate();
    let outer_subworkflow_node = Node {
        id: NodeId::generate(),
        executor: Box::new(SubWorkflowExecutor::new(inner_id)),
        inputs: vec![],
        outputs: vec![EdgeRef::new(outer_exit_edge)],
    };
    let outer_node_id = outer_subworkflow_node.id;
    let outer_plan = ExecutionPlan {
        nodes: vec![outer_subworkflow_node],
        edges: vec![Edge {
            id: outer_exit_edge,
            artifact_id: ArtifactId::new("outer-exit"),
            requires_deterministic: false,
        }],
        spec_index: HashMap::new(),
    };

    // ----- Run the outer scheduler.
    let scheduler = Scheduler::new(outer_registry, Arc::clone(&store), Arc::clone(&spine));
    let outer_plan_id = PlanId::generate();
    scheduler
        .run(&outer_plan_id, &outer_plan)
        .await
        .expect("outer plan must run");

    // ----- The outer SubWorkflow node terminated Completed.
    let states = store.node_states(&outer_plan_id).await.unwrap();
    assert_eq!(
        states.get(&outer_node_id),
        Some(&NodeState::Completed),
        "outer SubWorkflow node should be Completed",
    );

    // ----- The outer exit edge holds an artifact with Uncertain
    // provenance — the inner workflow's emit flowed through verbatim.
    // This is the runtime half of "provenance flows through" (ADR
    // 0011 § "Provenance flows through naturally", #357 verification
    // bullet 2).
    let outer_artifact_id = ArtifactId::new("outer-exit");
    let materialized = store
        .get_artifact(&outer_plan_id, &outer_artifact_id)
        .await
        .unwrap()
        .expect("outer exit artifact must be materialized");
    assert!(
        materialized.provenance.is_uncertain(),
        "expected Uncertain provenance on outer exit, got {:?}",
        materialized.provenance,
    );
    assert_eq!(
        materialized.provenance.source(),
        SourceTag::Agent,
        "inner Agent-sourced uncertainty should reach the outer artifact",
    );
}
