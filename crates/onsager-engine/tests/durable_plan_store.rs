//! Integration test for the durable [`SqlxPlanStore`] (B1, #499) and
//! restart-resume of a multi-node run (ADR 0023 decision 1).
//!
//! Skipped when `DATABASE_URL` is unset — matches the portal
//! integration-test convention. The execution-plan tables
//! (`crates/onsager-spine/migrations/006_execution_plans.sql`) are
//! ensured by `onsager_engine::scheduler::migrate::run`, so the test is
//! self-contained against a bare spine Postgres.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance};
use onsager_engine::nodes::{
    Executor, ExecutorContext, ExecutorError, ExecutorOutputs, ExecutorRegistry, NodeState, PlanId,
    PlanStore, SpineClient, SpineError,
};
use onsager_engine::scheduler::{PlanRunner, SqlxPlanStore};
use onsager_substrate::ids::EdgeId;
use onsager_substrate::ids::WorkflowId;
use onsager_substrate::spec_plan::{SpecDep, SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};
use onsager_substrate::{WorkflowLibraryError, WorkflowLookup};
use sqlx::PgPool;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

#[derive(Debug, Default)]
struct NoopSpine;

#[async_trait]
impl SpineClient for NoopSpine {
    async fn emit(&self, _: &str, _: serde_json::Value) -> Result<(), SpineError> {
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
        let art = Artifact::new(Kind::Document, "durable", "marvin", "test", vec![]);
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
                artifact_id: ArtifactId::new("art_in"),
                requires_deterministic: false,
            },
            Edge {
                id: exit,
                artifact_id: ArtifactId::new("art_out"),
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

struct TwoKindLibrary(Workflow, Workflow);

impl WorkflowLookup for TwoKindLibrary {
    fn get(&self, _: WorkflowId) -> Option<&Workflow> {
        None
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        match kind {
            "up" => Some(&self.0),
            "down" => Some(&self.1),
            _ => None,
        }
    }
}

fn two_spec_plan() -> SpecPlan {
    SpecPlan {
        specs: vec![
            SpecRef {
                id: SpecId::new("s1"),
                kind: "up".into(),
                inputs: Default::default(),
            },
            SpecRef {
                id: SpecId::new("s2"),
                kind: "down".into(),
                inputs: Default::default(),
            },
        ],
        deps: vec![SpecDep {
            from: SpecId::new("s1"),
            to: SpecId::new("s2"),
        }],
    }
}

fn build_runner(store: Arc<dyn PlanStore>) -> PlanRunner {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(EmitOne));
    PlanRunner::new(
        Arc::new(registry),
        store,
        Arc::new(NoopSpine) as Arc<dyn SpineClient>,
    )
}

async fn snapshot() -> Result<TwoKindLibrary, WorkflowLibraryError> {
    Ok(TwoKindLibrary(
        passthrough_workflow(),
        passthrough_workflow(),
    ))
}

#[tokio::test]
async fn durable_store_persists_node_state_and_resumes_after_restart() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_engine::scheduler::migrate::run(&pool)
        .await
        .expect("ensure execution-plan tables");

    let store: Arc<dyn PlanStore> = Arc::new(SqlxPlanStore::new(pool.clone()));
    let lib = snapshot().await.unwrap();
    let plan = two_spec_plan();
    // Unique plan id per test run so reruns don't collide.
    let plan_id = PlanId::new(format!("test-{}", uuid::Uuid::new_v4()));

    // First run: both nodes complete; state lands in the spine.
    build_runner(Arc::clone(&store))
        .run(&plan_id, &plan, &lib)
        .await
        .expect("first run completes");
    let states = store.node_states(&plan_id).await.unwrap();
    assert_eq!(states.len(), 2, "both nodes persisted");
    assert!(states.values().all(|s| *s == NodeState::Completed));

    // Simulate a crash mid-run: rewind the downstream node to Running
    // and clear nothing else. A *fresh* runner (no in-memory state)
    // sharing the same durable store must resume to completion — proof
    // the state was read from the spine, not memory.
    let some_node = *states.keys().next().expect("at least one node");
    store
        .set_node_state(&plan_id, some_node, NodeState::Running)
        .await
        .unwrap();

    build_runner(Arc::clone(&store))
        .run(&plan_id, &plan, &lib)
        .await
        .expect("resumed run completes");
    let resumed = store.node_states(&plan_id).await.unwrap();
    assert!(
        resumed.values().all(|s| *s == NodeState::Completed),
        "every node terminal after resume: {resumed:?}",
    );
}
