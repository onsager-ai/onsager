//! Substrate scheduler — runs an [`ExecutionPlan`] to completion.
//!
//! The scheduler replaces the 0.1 Forge tick loop: instead of polling
//! a status field, it walks the compiled plan's node graph, dispatches
//! each ready node through the [`ExecutorRegistry`], and persists state
//! transitions so a restart can resume mid-run.
//!
//! ## Where this lives
//!
//! RUN-01 (#359) names the module path `onsager-substrate/src/scheduler.rs`.
//! That placement is incompatible with the existing crate graph
//! (`onsager-nodes → onsager-substrate`): the scheduler must call
//! [`crate::dispatch`] / [`ExecutorRegistry`], which live in this
//! crate. Reversing the dep would cycle; abstracting dispatch behind a
//! trait in substrate would double the surface for no real callers.
//! The scheduler ships here; the spec text will be amended to match.
//!
//! ## Algorithm
//!
//! Per the RUN-01 description:
//!
//! 1. For each Pending node whose input edges are all Completed (or
//!    External + materialized in the [`PlanStore`]), transition it to
//!    `Ready` → `Running`, persist both transitions, then dispatch.
//! 2. On `Ok` outputs: persist each output artifact under its edge's
//!    `ArtifactId`, transition the node to `Completed`, emit
//!    `node.completed` on the spine.
//! 3. On `Err`: transition to `Failed`, emit `node.failed`, abort the
//!    plan with [`SchedulerError::NodeFailed`].
//! 4. Repeat until no progress (every node is terminal).
//!
//! Restart safety: at `run()` entry the scheduler reads the persisted
//! state map. `Completed` / `Failed` survive; `Running` / `Ready` are
//! reset to `Pending` so the node re-dispatches. Single-writer-per-
//! artifact (invariant 5, ADR 0018) is enforced by the compiler
//! (SUB-05), so a re-dispatch overwriting the same `ArtifactId` is
//! the same write — not a contention.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, NodeId};
use onsager_substrate::compiler::ExecutionPlan;
use onsager_substrate::ids::EdgeId;
use onsager_substrate::workflow::Node;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::warn;

use crate::context::ExecutorContext;
use crate::dispatch::dispatch;
use crate::error::ExecutorError;
use crate::registry::ExecutorRegistry;
use crate::spine::SpineClient;

// ---------------------------------------------------------------------------
// PlanId, NodeState
// ---------------------------------------------------------------------------

/// Externally-assigned identifier for one Execution Plan run.
///
/// Distinct from `WorkflowId` (a template) and `SpecId` (an input
/// node). Generated when a plan is handed to the scheduler; used as
/// the persistence key in [`PlanStore`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(String);

impl PlanId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Per-node lifecycle state, persisted in the future
/// `execution_plan_nodes` table (schema in the MIG-* family). The
/// scheduler writes a transition before *and* after dispatch so a
/// restart can tell the difference between "never dispatched" and
/// "dispatched but interrupted".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Some input edges still have an unresolved producer.
    Pending,
    /// All inputs are resolved — queued for dispatch. Emitted as
    /// `plan.node_ready` on the spine.
    Ready,
    /// `Executor::execute` is in flight.
    Running,
    /// Completed successfully — outputs persisted.
    Completed,
    /// Failed permanently. v1 does not retry.
    Failed,
}

impl NodeState {
    /// Did this state survive a restart, or does the scheduler need to
    /// reset and re-dispatch?
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

// ---------------------------------------------------------------------------
// PlanStore trait + in-memory impl
// ---------------------------------------------------------------------------

/// Persistence error from a [`PlanStore`] call. Free-text on purpose —
/// production implementations wrap sqlx errors, tests wrap nothing.
#[derive(Debug, Error)]
#[error("plan store error: {0}")]
pub struct PlanStoreError(pub String);

impl PlanStoreError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Persistence port for the scheduler.
///
/// Production binds this to a sqlx-backed adapter over the
/// `execution_plan_nodes` table (MIG-* family); tests use
/// [`InMemoryPlanStore`].
#[async_trait]
pub trait PlanStore: Send + Sync + std::fmt::Debug {
    /// Record a node's lifecycle transition. Idempotent — overwrites.
    async fn set_node_state(
        &self,
        plan_id: &PlanId,
        node_id: NodeId,
        state: NodeState,
    ) -> Result<(), PlanStoreError>;

    /// Read every known node state for a plan. An empty map means a
    /// fresh run; a non-empty map means recovery.
    async fn node_states(
        &self,
        plan_id: &PlanId,
    ) -> Result<HashMap<NodeId, NodeState>, PlanStoreError>;

    /// Persist a materialized artifact, keyed by
    /// `(plan_id, artifact_id)`. Idempotent — single-writer per
    /// `ArtifactId` is enforced at compile time (invariant 5).
    async fn put_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
        artifact: Artifact,
    ) -> Result<(), PlanStoreError>;

    /// Read an artifact previously written under this plan.
    async fn get_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
    ) -> Result<Option<Artifact>, PlanStoreError>;
}

/// In-memory implementation of [`PlanStore`]. Used by tests and the
/// early-bringup dispatcher binary before the SQL-backed store lands.
#[derive(Debug, Default)]
pub struct InMemoryPlanStore {
    states: Mutex<HashMap<(PlanId, NodeId), NodeState>>,
    artifacts: Mutex<HashMap<(PlanId, ArtifactId), Artifact>>,
}

impl InMemoryPlanStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PlanStore for InMemoryPlanStore {
    async fn set_node_state(
        &self,
        plan_id: &PlanId,
        node_id: NodeId,
        state: NodeState,
    ) -> Result<(), PlanStoreError> {
        self.states
            .lock()
            .await
            .insert((plan_id.clone(), node_id), state);
        Ok(())
    }

    async fn node_states(
        &self,
        plan_id: &PlanId,
    ) -> Result<HashMap<NodeId, NodeState>, PlanStoreError> {
        Ok(self
            .states
            .lock()
            .await
            .iter()
            .filter_map(|((p, n), s)| (p == plan_id).then_some((*n, *s)))
            .collect())
    }

    async fn put_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
        artifact: Artifact,
    ) -> Result<(), PlanStoreError> {
        self.artifacts
            .lock()
            .await
            .insert((plan_id.clone(), artifact_id.clone()), artifact);
        Ok(())
    }

    async fn get_artifact(
        &self,
        plan_id: &PlanId,
        artifact_id: &ArtifactId,
    ) -> Result<Option<Artifact>, PlanStoreError> {
        Ok(self
            .artifacts
            .lock()
            .await
            .get(&(plan_id.clone(), artifact_id.clone()))
            .cloned())
    }
}

// ---------------------------------------------------------------------------
// SchedulerError, Scheduler
// ---------------------------------------------------------------------------

/// Scheduler-side errors. Node-level failures keep the wrapped
/// [`ExecutorError`] for diagnostics.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// A node's executor returned `Err`. The plan is aborted.
    #[error("node {node_id} failed: {source}")]
    NodeFailed {
        node_id: NodeId,
        #[source]
        source: ExecutorError,
    },

    /// The plan reached a fixed point with non-terminal nodes still
    /// pending — typically because an upstream failed or the plan
    /// references an unresolvable input.
    #[error(
        "scheduler stuck: {pending} node(s) still pending after no \
         further progress could be made"
    )]
    Stuck { pending: usize },

    /// Persistence layer error.
    #[error("plan store error: {0}")]
    Store(String),
}

/// Event names the scheduler emits on the spine. Free constants so the
/// downstream `FactoryEventKind` migration (when these enter the typed
/// registry) can match by string.
pub const EVENT_NODE_READY: &str = "plan.node_ready";
pub const EVENT_NODE_RUNNING: &str = "plan.node_running";
pub const EVENT_NODE_COMPLETED: &str = "node.completed";
pub const EVENT_NODE_FAILED: &str = "node.failed";

/// The substrate scheduler.
///
/// Holds:
/// - `registry` — runtime executors, looked up by `executor_kind()`.
/// - `store` — node-state and artifact persistence.
/// - `spine` — port for emitting lifecycle events and for executors to
///   reach the event bus.
///
/// Construct once at startup; reuse across many plans.
#[derive(Clone)]
pub struct Scheduler {
    pub registry: Arc<ExecutorRegistry>,
    pub store: Arc<dyn PlanStore>,
    pub spine: Arc<dyn SpineClient>,
}

impl Scheduler {
    pub fn new(
        registry: Arc<ExecutorRegistry>,
        store: Arc<dyn PlanStore>,
        spine: Arc<dyn SpineClient>,
    ) -> Self {
        Self {
            registry,
            store,
            spine,
        }
    }

    /// Run `plan` to completion under `plan_id`. Restart-safe:
    /// non-terminal node states (`Ready` / `Running`) in the persisted
    /// map are reset to `Pending` and re-dispatched.
    pub async fn run(&self, plan_id: &PlanId, plan: &ExecutionPlan) -> Result<(), SchedulerError> {
        let prior = self
            .store
            .node_states(plan_id)
            .await
            .map_err(|e| SchedulerError::Store(e.0))?;

        // edge_id → producer node — built once, used for readiness checks.
        let producer_by_edge: HashMap<EdgeId, NodeId> = plan
            .nodes
            .iter()
            .flat_map(|n| n.outputs.iter().map(move |o| (o.edge_id, n.id)))
            .collect();

        // Seed in-process state from persisted state. Reset
        // non-terminal states so an interrupted run resumes from a
        // safe re-dispatch.
        let mut state: HashMap<NodeId, NodeState> = HashMap::new();
        for n in &plan.nodes {
            let s = prior
                .get(&n.id)
                .copied()
                .filter(|s| s.is_terminal())
                .unwrap_or(NodeState::Pending);
            state.insert(n.id, s);
        }

        loop {
            let mut made_progress = false;
            for node in &plan.nodes {
                if state.get(&node.id) != Some(&NodeState::Pending) {
                    continue;
                }
                if !self.inputs_ready(node, &state, &producer_by_edge) {
                    continue;
                }
                self.execute_node(plan_id, plan, node, &mut state).await?;
                made_progress = true;
            }
            if !made_progress {
                break;
            }
        }

        // Two distinct exits to surface:
        // - any node Failed (either before this run or from a prior
        //   one we recovered) — even if we didn't dispatch it, the
        //   plan is not Ok;
        // - any non-terminal node still pending — we made no further
        //   progress, the plan is stuck.
        if let Some((failed_id, _)) = state.iter().find(|(_, s)| **s == NodeState::Failed) {
            return Err(SchedulerError::NodeFailed {
                node_id: *failed_id,
                source: ExecutorError::failed("recovered failed state from prior run"),
            });
        }
        let pending = state
            .values()
            .filter(|s| **s != NodeState::Completed)
            .count();
        if pending > 0 {
            return Err(SchedulerError::Stuck { pending });
        }
        Ok(())
    }

    /// Are this node's input edges all satisfied? An input edge with
    /// no producer in the plan is External — treated as satisfied so
    /// the executor can read it from the [`PlanStore`] (or treat
    /// absence as empty input). Compile-time validation guarantees no
    /// dangling internal references.
    fn inputs_ready(
        &self,
        node: &Node,
        state: &HashMap<NodeId, NodeState>,
        producer_by_edge: &HashMap<EdgeId, NodeId>,
    ) -> bool {
        for input in &node.inputs {
            if let Some(producer) = producer_by_edge.get(&input.edge_id)
                && state.get(producer) != Some(&NodeState::Completed)
            {
                return false;
            }
        }
        true
    }

    async fn execute_node(
        &self,
        plan_id: &PlanId,
        plan: &ExecutionPlan,
        node: &Node,
        state: &mut HashMap<NodeId, NodeState>,
    ) -> Result<(), SchedulerError> {
        state.insert(node.id, NodeState::Ready);
        self.transition(plan_id, node.id, NodeState::Ready).await?;

        state.insert(node.id, NodeState::Running);
        self.transition(plan_id, node.id, NodeState::Running)
            .await?;

        let inputs = self.gather_inputs(plan_id, node, plan).await?;
        let ctx = ExecutorContext {
            node_id: node.id,
            inputs,
            spine: Arc::clone(&self.spine),
            // Read the per-node SubWorkflow ref off the substrate
            // executor — the registered runtime executor only sees
            // the kind string via dispatch, so this is how the
            // SubWorkflow runtime (#357) learns which workflow to
            // run for *this* node.
            subworkflow_ref: node.executor.subworkflow_ref(),
        };
        match dispatch(&self.registry, node, ctx).await {
            Ok(outputs) => {
                self.persist_outputs(plan_id, plan, node, outputs).await?;
                state.insert(node.id, NodeState::Completed);
                self.transition(plan_id, node.id, NodeState::Completed)
                    .await?;
                Ok(())
            }
            Err(err) => {
                state.insert(node.id, NodeState::Failed);
                self.transition(plan_id, node.id, NodeState::Failed).await?;
                Err(SchedulerError::NodeFailed {
                    node_id: node.id,
                    source: err,
                })
            }
        }
    }

    async fn gather_inputs(
        &self,
        plan_id: &PlanId,
        node: &Node,
        plan: &ExecutionPlan,
    ) -> Result<Vec<(ArtifactId, Artifact)>, SchedulerError> {
        let mut inputs = Vec::new();
        for input_ref in &node.inputs {
            let edge = plan
                .edges
                .iter()
                .find(|e| e.id == input_ref.edge_id)
                .expect("compile-time validation guarantees every input edge resolves");
            if let Some(artifact) = self
                .store
                .get_artifact(plan_id, &edge.artifact_id)
                .await
                .map_err(|e| SchedulerError::Store(e.0))?
            {
                inputs.push((edge.artifact_id.clone(), artifact));
            }
        }
        Ok(inputs)
    }

    async fn persist_outputs(
        &self,
        plan_id: &PlanId,
        plan: &ExecutionPlan,
        node: &Node,
        outputs: crate::context::ExecutorOutputs,
    ) -> Result<(), SchedulerError> {
        // Contract: an executor returns *at most* one artifact per
        // declared output edge, in declaration order. Side-effect-
        // only executors (NoOp) legitimately return zero; surface
        // *extra* outputs as a bug (silently dropping them hid
        // executor wiring mistakes). v1 leaves "Completed with
        // fewer artifacts than declared" as the executor's
        // contract with its downstream consumers — see the reply
        // on #381 line 462 for the trade-off.
        if outputs.artifacts.len() > node.outputs.len() {
            return Err(SchedulerError::NodeFailed {
                node_id: node.id,
                source: ExecutorError::failed(format!(
                    "executor returned {} artifact(s) but node declares only {} output edge(s)",
                    outputs.artifacts.len(),
                    node.outputs.len(),
                )),
            });
        }
        for (i, (_, mut artifact)) in outputs.artifacts.into_iter().enumerate() {
            let output_ref = node.outputs[i];
            let edge = plan
                .edges
                .iter()
                .find(|e| e.id == output_ref.edge_id)
                .expect("compile-time validation guarantees every output edge resolves");
            // Align the artifact's own id with the edge's
            // (compile-time-namespaced) ArtifactId so the persisted
            // key and the artifact body agree. Existing executors
            // (Script, Verify) generate fresh ULIDs in
            // `Artifact::new`; the scheduler is the only layer that
            // knows the edge mapping, so reconciliation happens here.
            artifact.artifact_id = edge.artifact_id.clone();
            self.store
                .put_artifact(plan_id, &edge.artifact_id, artifact)
                .await
                .map_err(|e| SchedulerError::Store(e.0))?;
        }
        Ok(())
    }

    async fn transition(
        &self,
        plan_id: &PlanId,
        node_id: NodeId,
        state: NodeState,
    ) -> Result<(), SchedulerError> {
        self.store
            .set_node_state(plan_id, node_id, state)
            .await
            .map_err(|e| SchedulerError::Store(e.0))?;
        let kind = match state {
            NodeState::Ready => EVENT_NODE_READY,
            NodeState::Running => EVENT_NODE_RUNNING,
            NodeState::Completed => EVENT_NODE_COMPLETED,
            NodeState::Failed => EVENT_NODE_FAILED,
            NodeState::Pending => return Ok(()),
        };
        let payload = serde_json::json!({
            "plan_id": plan_id.as_str(),
            "node_id": node_id,
        });
        if let Err(e) = self.spine.emit(kind, payload).await {
            warn!(plan = %plan_id, node = %node_id, "spine emit failed: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::{ArtifactId, Kind};
    use onsager_substrate::compiler::ExecutionPlan;
    use onsager_substrate::ids::EdgeId;
    use onsager_substrate::workflow::{Edge, EdgeRef, Node as SubstrateNode};
    use std::collections::HashMap;

    /// Build a linear two-node plan: [A] → edge → [B].
    fn linear_plan() -> ExecutionPlan {
        let edge_mid = EdgeId::generate();
        let edge_out = EdgeId::generate();
        let a = SubstrateNode {
            id: NodeId::generate(),
            executor: Box::new(onsager_substrate::executor::NoOpExecutor),
            inputs: vec![],
            outputs: vec![EdgeRef::new(edge_mid)],
        };
        let b = SubstrateNode {
            id: NodeId::generate(),
            executor: Box::new(onsager_substrate::executor::NoOpExecutor),
            inputs: vec![EdgeRef::new(edge_mid)],
            outputs: vec![EdgeRef::new(edge_out)],
        };
        ExecutionPlan {
            nodes: vec![a, b],
            edges: vec![
                Edge {
                    id: edge_mid,
                    artifact_id: ArtifactId::new("mid"),
                    requires_deterministic: false,
                },
                Edge {
                    id: edge_out,
                    artifact_id: ArtifactId::new("out"),
                    requires_deterministic: false,
                },
            ],
            spec_index: HashMap::new(),
        }
    }

    fn fresh_scheduler() -> (Scheduler, Arc<InMemoryPlanStore>, Arc<MockSpine>) {
        let registry = Arc::new(ExecutorRegistry::with_noop());
        let store = Arc::new(InMemoryPlanStore::new());
        let spine = Arc::new(MockSpine::default());
        let scheduler = Scheduler::new(
            registry,
            Arc::clone(&store) as Arc<dyn PlanStore>,
            Arc::clone(&spine) as Arc<dyn SpineClient>,
        );
        (scheduler, store, spine)
    }

    #[tokio::test]
    async fn linear_plan_runs_all_nodes_to_completion() {
        let (scheduler, store, spine) = fresh_scheduler();
        let plan = linear_plan();
        let plan_id = PlanId::generate();

        scheduler.run(&plan_id, &plan).await.unwrap();

        // Every node terminated as Completed.
        let states = store.node_states(&plan_id).await.unwrap();
        assert_eq!(states.len(), 2);
        for (_, state) in states {
            assert_eq!(state, NodeState::Completed);
        }
        // Each Completed node emitted at least one node.completed.
        let kinds: Vec<_> = spine
            .emitted
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(
            kinds.iter().filter(|k| *k == EVENT_NODE_COMPLETED).count(),
            2,
        );
        // The Ready / Running transitions emitted too.
        assert!(kinds.iter().any(|k| k == EVENT_NODE_READY));
        assert!(kinds.iter().any(|k| k == EVENT_NODE_RUNNING));
    }

    #[tokio::test]
    async fn restart_resumes_from_persisted_state() {
        // Simulate an interrupted run: pre-populate the store with
        // one Completed node and one Running node. The scheduler must
        // skip the Completed node, reset Running → Pending, and
        // re-dispatch.
        let (scheduler, store, _) = fresh_scheduler();
        let plan = linear_plan();
        let plan_id = PlanId::generate();

        let a_id = plan.nodes[0].id;
        let b_id = plan.nodes[1].id;
        store
            .set_node_state(&plan_id, a_id, NodeState::Completed)
            .await
            .unwrap();
        // Persist A's output so B has its input available on resume.
        store
            .put_artifact(
                &plan_id,
                &plan.edges[0].artifact_id,
                Artifact::new(Kind::Document, "fixture", "marvin", "test", vec![]),
            )
            .await
            .unwrap();
        store
            .set_node_state(&plan_id, b_id, NodeState::Running)
            .await
            .unwrap();

        scheduler.run(&plan_id, &plan).await.unwrap();

        // Both nodes terminal; B re-ran (now Completed).
        let states = store.node_states(&plan_id).await.unwrap();
        assert_eq!(states.get(&a_id), Some(&NodeState::Completed));
        assert_eq!(states.get(&b_id), Some(&NodeState::Completed));
    }

    /// Failing executor: registers under the `noop` kind so it
    /// overrides the default `NoOpExecutor` in the registry.
    #[derive(Debug)]
    struct FailingExecutor;

    #[async_trait]
    impl crate::Executor for FailingExecutor {
        fn executor_kind(&self) -> &'static str {
            "noop"
        }
        fn declared_provenance(
            &self,
            _: &[onsager_artifact::Provenance],
        ) -> onsager_artifact::Provenance {
            onsager_artifact::Provenance::external_deterministic()
        }
        async fn execute(
            &self,
            _: ExecutorContext,
        ) -> Result<crate::context::ExecutorOutputs, ExecutorError> {
            Err(ExecutorError::failed("boom"))
        }
    }

    #[tokio::test]
    async fn failing_node_aborts_plan_and_persists_failed() {
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(FailingExecutor));
        let registry = Arc::new(registry);
        let store = Arc::new(InMemoryPlanStore::new());
        let spine = Arc::new(MockSpine::default());
        let scheduler = Scheduler::new(
            registry,
            Arc::clone(&store) as Arc<dyn PlanStore>,
            Arc::clone(&spine) as Arc<dyn SpineClient>,
        );

        let plan = linear_plan();
        let plan_id = PlanId::generate();
        let err = scheduler.run(&plan_id, &plan).await.unwrap_err();
        assert!(matches!(err, SchedulerError::NodeFailed { .. }));

        // The first node persisted Failed; the second never ran.
        let states = store.node_states(&plan_id).await.unwrap();
        let failed = states
            .values()
            .filter(|s| matches!(s, NodeState::Failed))
            .count();
        assert_eq!(failed, 1);
        // node.failed event was emitted.
        let kinds: Vec<_> = spine
            .emitted
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert!(kinds.iter().any(|k| k == EVENT_NODE_FAILED));
    }

    #[tokio::test]
    async fn pre_existing_failed_node_surfaces_as_node_failed() {
        // Recovery scenario: a prior run left a node Failed. On
        // re-run, the scheduler must NOT report Ok just because no
        // dispatchable work remains — it must surface the persisted
        // failure (Copilot review on #381, line 359).
        let (scheduler, store, _) = fresh_scheduler();
        let plan = linear_plan();
        let plan_id = PlanId::generate();
        let a_id = plan.nodes[0].id;
        store
            .set_node_state(&plan_id, a_id, NodeState::Failed)
            .await
            .unwrap();

        let err = scheduler.run(&plan_id, &plan).await.unwrap_err();
        assert!(
            matches!(err, SchedulerError::NodeFailed { node_id, .. } if node_id == a_id),
            "expected NodeFailed(a), got: {err:?}",
        );
    }
}
