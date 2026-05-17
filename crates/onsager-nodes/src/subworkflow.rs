//! [`SubWorkflowExecutor`] (EXE-05, issue #357) — a node whose
//! "behavior" is *another workflow*.
//!
//! Per [ADR 0011](../../../docs/adr/0011-subworkflow-implements-vsm-recursion.md),
//! workflow nesting is a property of the executor type, not a separate
//! construct: a SubWorkflow node carries a `workflow_ref: WorkflowId`,
//! and at run time the runtime resolves that reference in the
//! [`WorkflowLibrary`](onsager_substrate::library::WorkflowLibrary)
//! and runs the inner workflow with the SubWorkflow node's inputs
//! bound to its entry edges. The inner workflow's exit artifacts —
//! provenance and all — become the SubWorkflow node's outputs.
//!
//! ## Both halves of the [Executor trait pair](crate::executor)
//!
//! Like [`crate::VerifyExecutor`] and [`crate::AgentExecutor`], the
//! substrate-side trait and the runtime-side trait are both
//! implemented on the same `SubWorkflowExecutor` struct:
//!
//! - **substrate side** ([`onsager_substrate::executor::Executor`]):
//!   serializable via [`typetag`] under `kind = "subworkflow"`,
//!   declares `subworkflow_ref()` so the kernel's invariant 4 (ADR
//!   0018) catches unresolved references and cycles statically.
//! - **runtime side** ([`crate::Executor`]): looks the workflow up,
//!   delegates to a [`SubWorkflowRunner`], and packages the inner
//!   outputs.
//!
//! ## Per-node config in a registry-based dispatch world
//!
//! The runtime instance lives in [`crate::ExecutorRegistry`] keyed by
//! `executor_kind() == "subworkflow"` — one instance for *every*
//! SubWorkflow node in any workflow. The per-node `workflow_ref` lives
//! on the *substrate-side* executor inside each node, so the
//! [`crate::Scheduler`] reads `node.executor.subworkflow_ref()` and
//! threads it through [`crate::ExecutorContext::subworkflow_ref`]
//! before dispatch. The runtime instance reads the ref off the
//! context, not off itself.
//!
//! ## Provenance flows through naturally
//!
//! The inner workflow's exit artifacts carry whatever provenance the
//! inner nodes emit (invariant 2's max-uncertainty rule applies at
//! every depth). The SubWorkflow executor returns those artifacts
//! unchanged: a `Verify` inside the inner workflow upgrades exactly
//! the same way it would upgrade at the outer level (ADR 0011 §
//! "Provenance flows through naturally"). The substrate-side
//! `declared_provenance` is conservative — it propagates the worst
//! input provenance — so static validation never claims the
//! SubWorkflow upgrades anything on its own.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Provenance};
use onsager_substrate::compiler::ExecutionPlan;
use onsager_substrate::executor::Executor as SubstrateExecutor;
use onsager_substrate::ids::WorkflowId;
use onsager_substrate::library::WorkflowLibrary;
use onsager_substrate::spec_plan::SpecId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor as RuntimeExecutor;
use crate::registry::ExecutorRegistry;
use crate::scheduler::{InMemoryPlanStore, PlanId, PlanStore, Scheduler};
use crate::spine::SpineClient;

/// Wire-format tag for the SubWorkflow executor. Shared by the
/// substrate typetag discriminator and the runtime registry key.
pub const SUBWORKFLOW_KIND: &str = "subworkflow";

/// A node whose behavior is another workflow.
///
/// The `workflow_ref` field is the per-node configuration the kernel
/// validates against the library (invariant 4); the `runner` is the
/// runtime port that actually executes inner workflows. `runner` is
/// `#[serde(skip)]` — it's a runtime wiring concern, not part of the
/// workflow template's serialized shape. A deserialized
/// `SubWorkflowExecutor` carries [`UnconfiguredRunner`] and errors at
/// `execute` time until [`SubWorkflowExecutor::with_runner`] rewires
/// it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubWorkflowExecutor {
    /// The inner workflow's id in the [`WorkflowLibrary`].
    /// [Invariant 4](onsager_substrate::validate) checks that this
    /// resolves and is acyclic before the scheduler runs.
    pub workflow_ref: WorkflowId,

    #[serde(skip, default = "default_runner")]
    runner: Arc<dyn SubWorkflowRunner>,
}

fn default_runner() -> Arc<dyn SubWorkflowRunner> {
    Arc::new(UnconfiguredRunner)
}

impl SubWorkflowExecutor {
    /// Build a SubWorkflow executor pointing at `workflow_ref`. Runner
    /// defaults to [`UnconfiguredRunner`]; layer the real runner in
    /// with [`Self::with_runner`] before registering.
    pub fn new(workflow_ref: WorkflowId) -> Self {
        Self {
            workflow_ref,
            runner: default_runner(),
        }
    }

    /// Replace the runtime runner. Required before `execute` does
    /// anything useful — the default errors loudly.
    pub fn with_runner(mut self, runner: Arc<dyn SubWorkflowRunner>) -> Self {
        self.runner = runner;
        self
    }
}

// ---------------------------------------------------------------------------
// Runner abstraction
// ---------------------------------------------------------------------------

/// Why a [`SubWorkflowRunner::run`] call failed.
#[derive(Debug, Error)]
#[error("subworkflow runner error: {0}")]
pub struct SubWorkflowRunError(String);

impl SubWorkflowRunError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Port over the actual sub-workflow execution backend.
///
/// Production wiring uses [`SchedulerSubWorkflowRunner`] — it
/// resolves the [`WorkflowId`] via a [`WorkflowLibrary`], instantiates
/// the workflow into an [`ExecutionPlan`], binds the inputs onto the
/// inner workflow's entry edges via a [`PlanStore`], runs a nested
/// [`Scheduler`], and collects the exit artifacts. Tests substitute
/// [`StubSubWorkflowRunner`] for a canned response without spinning
/// up an inner run.
///
/// Object-safe by design: held inside [`SubWorkflowExecutor`] as
/// `Arc<dyn SubWorkflowRunner>`.
#[async_trait]
pub trait SubWorkflowRunner: Send + Sync + std::fmt::Debug {
    /// Run the workflow referenced by `workflow_ref`, with `inputs`
    /// bound onto its entry edges (positionally), and return the
    /// inner workflow's exit artifacts in the order its `output_specs`
    /// declares them.
    async fn run(
        &self,
        workflow_ref: WorkflowId,
        inputs: Vec<(ArtifactId, Artifact)>,
    ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError>;
}

/// Default runner placeholder.
///
/// Carried by every freshly-deserialized `SubWorkflowExecutor` (the
/// `runner` field is `#[serde(skip)]`). Calling `run` always errors;
/// substrate-side validation does not touch the runner, so it's
/// invisible to invariant 4 / compile-time checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnconfiguredRunner;

#[async_trait]
impl SubWorkflowRunner for UnconfiguredRunner {
    async fn run(
        &self,
        _workflow_ref: WorkflowId,
        _inputs: Vec<(ArtifactId, Artifact)>,
    ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
        Err(SubWorkflowRunError::new(
            "SubWorkflowExecutor has no configured runner — call `with_runner(..)` before registering",
        ))
    }
}

/// In-memory runner for tests and early-bringup wiring. Returns the
/// configured `outputs` for every call regardless of inputs.
#[derive(Debug, Clone)]
pub struct StubSubWorkflowRunner {
    pub outputs: Vec<(ArtifactId, Artifact)>,
}

impl StubSubWorkflowRunner {
    pub fn new(outputs: Vec<(ArtifactId, Artifact)>) -> Self {
        Self { outputs }
    }
}

#[async_trait]
impl SubWorkflowRunner for StubSubWorkflowRunner {
    async fn run(
        &self,
        _workflow_ref: WorkflowId,
        _inputs: Vec<(ArtifactId, Artifact)>,
    ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
        Ok(self.outputs.clone())
    }
}

// ---------------------------------------------------------------------------
// SchedulerSubWorkflowRunner — production-shape runner
// ---------------------------------------------------------------------------

/// The production [`SubWorkflowRunner`]: resolves `workflow_ref`
/// through a [`WorkflowLibrary`], instantiates the inner workflow into
/// a self-contained [`ExecutionPlan`], binds inputs onto its entry
/// edges, runs a nested [`Scheduler`], and reads the exit artifacts
/// back out of the [`PlanStore`].
///
/// The library type is held behind `Arc<dyn WorkflowLibrary + Send +
/// Sync>` so a single concrete library implementation can be shared
/// across the outer scheduler and any nested SubWorkflow runs.
///
/// The executor registry, plan store, and spine client are the same
/// shape the outer [`Scheduler`] uses, so a SubWorkflow run looks
/// identical to a top-level run from the spine's point of view.
pub struct SchedulerSubWorkflowRunner {
    library: Arc<dyn WorkflowLibrary + Send + Sync>,
    registry: Arc<ExecutorRegistry>,
    store: Arc<dyn PlanStore>,
    spine: Arc<dyn SpineClient>,
}

impl std::fmt::Debug for SchedulerSubWorkflowRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `WorkflowLibrary` is not `Debug`-bounded; report just the
        // pointer-equivalent identity so logs can tell two runners
        // apart without forcing every library impl to implement
        // `Debug`.
        f.debug_struct("SchedulerSubWorkflowRunner")
            .field("registry", &self.registry)
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl SchedulerSubWorkflowRunner {
    /// Build a runner. The same library / registry / store / spine
    /// the outer scheduler is using are the right choice — a nested
    /// run reuses the outer infrastructure rather than minting fresh
    /// copies.
    pub fn new(
        library: Arc<dyn WorkflowLibrary + Send + Sync>,
        registry: Arc<ExecutorRegistry>,
        store: Arc<dyn PlanStore>,
        spine: Arc<dyn SpineClient>,
    ) -> Self {
        Self {
            library,
            registry,
            store,
            spine,
        }
    }

    /// A runner with a fresh [`InMemoryPlanStore`] — useful for
    /// tests that don't want to share a store with the outer run.
    pub fn with_in_memory_store(
        library: Arc<dyn WorkflowLibrary + Send + Sync>,
        registry: Arc<ExecutorRegistry>,
        spine: Arc<dyn SpineClient>,
    ) -> Self {
        Self::new(library, registry, Arc::new(InMemoryPlanStore::new()), spine)
    }
}

#[async_trait]
impl SubWorkflowRunner for SchedulerSubWorkflowRunner {
    async fn run(
        &self,
        workflow_ref: WorkflowId,
        inputs: Vec<(ArtifactId, Artifact)>,
    ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
        // Resolve the inner workflow. Invariant 4 should have caught
        // any unresolved reference at compile time, but the runtime
        // re-checks because a runtime-only callsite may have skipped
        // validation (a hand-built SubWorkflowExecutor pointed at an
        // empty library, for instance).
        let workflow = self.library.get(workflow_ref).ok_or_else(|| {
            SubWorkflowRunError::new(format!("workflow {workflow_ref} not registered in library"))
        })?;

        // Namespace the inner run with a synthetic SpecId — UUID v4
        // keyed so two concurrent sub-runs of the same workflow don't
        // collide on the per-spec namespace `Workflow::instantiate`
        // derives. The outer plan's SpecId space is untouched.
        let spec_id = SpecId::new(format!("sub-{}", uuid::Uuid::new_v4()));
        let inst = workflow.instantiate(&spec_id);

        // The inner Execution Plan is just the instantiated nodes +
        // edges; no cross-spec wiring because SubWorkflow runs are
        // self-contained. `spec_index` stays empty — observers walk
        // the outer plan for spec attribution.
        let plan = ExecutionPlan {
            nodes: inst.nodes,
            edges: inst.edges,
            spec_index: HashMap::new(),
        };
        let exit_edges = inst.exit_edges.clone();
        let entry_edge_ids = inst.entry_edges.clone();

        // Bind inputs onto entry edges positionally. v1 ADR 0015
        // fixes workflows at single-entry / single-exit, so this
        // simple zip is sufficient; widening the IO model is a
        // follow-up. Mismatched lengths are surfaced as a runner
        // error so the operator sees the wiring problem instead of
        // a silent drop.
        if inputs.len() > entry_edge_ids.len() {
            return Err(SubWorkflowRunError::new(format!(
                "SubWorkflow received {} input(s) but workflow declares only {} entry edge(s)",
                inputs.len(),
                entry_edge_ids.len(),
            )));
        }
        let plan_id = PlanId::generate();
        for ((_input_id, input_artifact), entry_edge_id) in
            inputs.into_iter().zip(entry_edge_ids.iter())
        {
            let edge = plan
                .edges
                .iter()
                .find(|e| e.id == *entry_edge_id)
                .ok_or_else(|| {
                    SubWorkflowRunError::new(format!(
                        "entry edge {entry_edge_id} missing from instantiated plan",
                    ))
                })?;
            self.store
                .put_artifact(&plan_id, &edge.artifact_id, input_artifact)
                .await
                .map_err(|e| SubWorkflowRunError::new(e.0))?;
        }

        // Run a nested scheduler. The same registry / store / spine
        // — a nested run is the same shape from the spine's point
        // of view, modulo the plan_id discriminator.
        let scheduler = Scheduler::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.store),
            Arc::clone(&self.spine),
        );
        scheduler
            .run(&plan_id, &plan)
            .await
            .map_err(|e| SubWorkflowRunError::new(e.to_string()))?;

        // Collect outputs from the inner workflow's exit edges. The
        // artifacts carry whatever provenance the inner producers
        // emitted — that's how Uncertain inner outputs reach the
        // outer plan as Uncertain outer outputs (ADR 0011 §
        // "Provenance flows through naturally").
        let mut outputs = Vec::new();
        for output_spec in &exit_edges {
            let edge = plan
                .edges
                .iter()
                .find(|e| e.id == output_spec.edge_id)
                .ok_or_else(|| {
                    SubWorkflowRunError::new(format!(
                        "exit edge {} missing from instantiated plan",
                        output_spec.edge_id
                    ))
                })?;
            let Some(artifact) = self
                .store
                .get_artifact(&plan_id, &edge.artifact_id)
                .await
                .map_err(|e| SubWorkflowRunError::new(e.0))?
            else {
                return Err(SubWorkflowRunError::new(format!(
                    "exit edge {} ({}) was not produced by the inner workflow",
                    output_spec.edge_id, edge.artifact_id,
                )));
            };
            outputs.push((edge.artifact_id.clone(), artifact));
        }
        Ok(outputs)
    }
}

// ---------------------------------------------------------------------------
// Substrate side — typetag + invariant 4 hook
// ---------------------------------------------------------------------------

#[typetag::serde(name = "subworkflow")]
impl SubstrateExecutor for SubWorkflowExecutor {
    fn executor_kind(&self) -> &'static str {
        SUBWORKFLOW_KIND
    }

    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance {
        // Conservative static declaration: propagate the worst input
        // provenance. Static validation does not recurse through
        // `workflow_ref`, so the SubWorkflow can't *statically* know
        // whether the inner workflow will upgrade to Deterministic
        // via a `Verify` node or stay Uncertain. The actual emit at
        // runtime is whatever the inner workflow's terminal node
        // produces (see `SchedulerSubWorkflowRunner::run`); invariant
        // 2's max-uncertainty rule then keeps the outer Plan honest.
        inputs
            .iter()
            .copied()
            .find(Provenance::is_uncertain)
            .unwrap_or_default()
    }

    fn subworkflow_ref(&self) -> Option<WorkflowId> {
        Some(self.workflow_ref)
    }
}

// ---------------------------------------------------------------------------
// Runtime side — async execute via the runner
// ---------------------------------------------------------------------------

#[async_trait]
impl RuntimeExecutor for SubWorkflowExecutor {
    fn executor_kind(&self) -> &'static str {
        SUBWORKFLOW_KIND
    }

    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance {
        SubstrateExecutor::declared_provenance(self, inputs)
    }

    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        // The registered runtime instance is shared across every
        // SubWorkflow node — its own `workflow_ref` is a placeholder.
        // The scheduler reads the per-node ref off the substrate
        // executor and threads it through the context; we prefer
        // that. Fall back to `self.workflow_ref` for the direct-
        // execute path (tests, and any caller that bypasses the
        // scheduler).
        let workflow_ref = ctx.subworkflow_ref.unwrap_or(self.workflow_ref);
        let inputs = ctx.inputs.clone();
        let outputs = self
            .runner
            .run(workflow_ref, inputs)
            .await
            .map_err(|e| ExecutorError::failed(e.to_string()))?;
        Ok(ExecutorOutputs { artifacts: outputs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::{Kind, NodeId, SourceTag};

    fn ctx_for(node_id: NodeId, inputs: Vec<(ArtifactId, Artifact)>) -> ExecutorContext {
        ExecutorContext {
            node_id,
            inputs,
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        }
    }

    fn ctx_for_with_ref(
        node_id: NodeId,
        inputs: Vec<(ArtifactId, Artifact)>,
        workflow_ref: WorkflowId,
    ) -> ExecutorContext {
        ExecutorContext {
            node_id,
            inputs,
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: Some(workflow_ref),
        }
    }

    fn uncertain_artifact() -> Artifact {
        let mut art = Artifact::new(Kind::Code, "from-agent", "marvin", "agent", vec![]);
        art.provenance = Provenance::Uncertain {
            source: SourceTag::Agent,
        };
        art
    }

    fn deterministic_artifact() -> Artifact {
        Artifact::new(Kind::Document, "from-script", "marvin", "test", vec![])
    }

    // -----------------------------------------------------------------
    // Substrate side — typetag, kind, subworkflow_ref, declared.
    // -----------------------------------------------------------------

    #[test]
    fn executor_kind_is_subworkflow_on_both_traits() {
        let exec = SubWorkflowExecutor::new(WorkflowId::generate());
        assert_eq!(SubstrateExecutor::executor_kind(&exec), SUBWORKFLOW_KIND);
        assert_eq!(RuntimeExecutor::executor_kind(&exec), SUBWORKFLOW_KIND);
        assert_eq!(SUBWORKFLOW_KIND, "subworkflow");
    }

    #[test]
    fn subworkflow_ref_returns_the_wrapped_workflow_id() {
        let id = WorkflowId::generate();
        let exec = SubWorkflowExecutor::new(id);
        // The substrate trait's hook is what invariant 4 reads.
        assert_eq!(
            <SubWorkflowExecutor as SubstrateExecutor>::subworkflow_ref(&exec),
            Some(id),
        );
    }

    #[test]
    fn declared_provenance_propagates_worst_input() {
        let exec = SubWorkflowExecutor::new(WorkflowId::generate());
        // No inputs → external-deterministic default.
        assert_eq!(
            RuntimeExecutor::declared_provenance(&exec, &[]),
            Provenance::external_deterministic(),
        );
        // Mixed inputs → the Uncertain one wins.
        let p = RuntimeExecutor::declared_provenance(
            &exec,
            &[
                Provenance::Deterministic {
                    source: SourceTag::Script,
                },
                Provenance::Uncertain {
                    source: SourceTag::Agent,
                },
            ],
        );
        assert!(p.is_uncertain());
        assert_eq!(p.source(), SourceTag::Agent);
        // Substrate side mirrors the runtime side.
        assert_eq!(
            SubstrateExecutor::declared_provenance(&exec, &[]),
            RuntimeExecutor::declared_provenance(&exec, &[]),
        );
    }

    #[test]
    fn subworkflow_executor_roundtrips_as_substrate_trait_object() {
        let original_ref = WorkflowId::generate();
        let original: Box<dyn SubstrateExecutor> = Box::new(SubWorkflowExecutor::new(original_ref));
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json["kind"], "subworkflow");
        // workflow_ref serializes verbatim (it's a transparent UUID).
        assert_eq!(
            json["workflow_ref"].as_str().unwrap(),
            original_ref.to_string(),
        );

        let roundtrip: Box<dyn SubstrateExecutor> = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.executor_kind(), "subworkflow");
        assert_eq!(roundtrip.subworkflow_ref(), Some(original_ref));
    }

    // -----------------------------------------------------------------
    // Runtime side — runner wiring + provenance pass-through.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_without_runner_errors_clearly() {
        let exec = SubWorkflowExecutor::new(WorkflowId::generate());
        let err = exec
            .execute(ctx_for(NodeId::generate(), vec![]))
            .await
            .expect_err("UnconfiguredRunner must fail");
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("with_runner"), "message: {msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_returns_runner_outputs_unchanged() {
        // The runner's outputs flow through the executor verbatim —
        // including the artifact's provenance. This is the runtime
        // half of ADR 0011's "provenance flows through" guarantee.
        let stub_id = ArtifactId::new("out-from-stub");
        let stub = Arc::new(StubSubWorkflowRunner::new(vec![(
            stub_id.clone(),
            uncertain_artifact(),
        )]));
        let exec = SubWorkflowExecutor::new(WorkflowId::generate()).with_runner(stub);
        let outputs = exec
            .execute(ctx_for(NodeId::generate(), vec![]))
            .await
            .unwrap();
        assert_eq!(outputs.artifacts.len(), 1);
        let (id, art) = &outputs.artifacts[0];
        assert_eq!(id, &stub_id);
        assert!(art.provenance.is_uncertain());
        assert_eq!(art.provenance.source(), SourceTag::Agent);
    }

    #[tokio::test]
    async fn execute_prefers_subworkflow_ref_from_context() {
        // Verification: the scheduler passes the *per-node* ref via
        // the context. A runner that captures the workflow_ref it
        // was called with proves the context wins over the registered
        // instance's placeholder ref.
        #[derive(Debug, Default)]
        struct CapturingRunner {
            seen: std::sync::Mutex<Option<WorkflowId>>,
        }
        #[async_trait]
        impl SubWorkflowRunner for CapturingRunner {
            async fn run(
                &self,
                workflow_ref: WorkflowId,
                _inputs: Vec<(ArtifactId, Artifact)>,
            ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
                *self.seen.lock().unwrap() = Some(workflow_ref);
                Ok(vec![])
            }
        }
        let registered_ref = WorkflowId::generate(); // placeholder
        let per_node_ref = WorkflowId::generate(); // the one the scheduler threads
        let capturing: Arc<CapturingRunner> = Arc::new(CapturingRunner::default());
        let exec = SubWorkflowExecutor::new(registered_ref).with_runner(capturing.clone());
        exec.execute(ctx_for_with_ref(NodeId::generate(), vec![], per_node_ref))
            .await
            .unwrap();
        assert_eq!(*capturing.seen.lock().unwrap(), Some(per_node_ref));
    }

    #[tokio::test]
    async fn execute_falls_back_to_self_workflow_ref_when_context_is_none() {
        // Direct-execute path (tests, callers that bypass the
        // scheduler): no context ref → use the one on the executor.
        #[derive(Debug, Default)]
        struct CapturingRunner {
            seen: std::sync::Mutex<Option<WorkflowId>>,
        }
        #[async_trait]
        impl SubWorkflowRunner for CapturingRunner {
            async fn run(
                &self,
                workflow_ref: WorkflowId,
                _inputs: Vec<(ArtifactId, Artifact)>,
            ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
                *self.seen.lock().unwrap() = Some(workflow_ref);
                Ok(vec![])
            }
        }
        let self_ref = WorkflowId::generate();
        let capturing: Arc<CapturingRunner> = Arc::new(CapturingRunner::default());
        let exec = SubWorkflowExecutor::new(self_ref).with_runner(capturing.clone());
        exec.execute(ctx_for(NodeId::generate(), vec![]))
            .await
            .unwrap();
        assert_eq!(*capturing.seen.lock().unwrap(), Some(self_ref));
    }

    // -----------------------------------------------------------------
    // Compile-time check: object-safe on the runtime trait.
    // -----------------------------------------------------------------

    #[test]
    fn subworkflow_executor_trait_object_safe() {
        let _boxed: Box<dyn RuntimeExecutor> =
            Box::new(SubWorkflowExecutor::new(WorkflowId::generate()));
        let _arced: Arc<dyn RuntimeExecutor> =
            Arc::new(SubWorkflowExecutor::new(WorkflowId::generate()));
    }

    // -----------------------------------------------------------------
    // Dispatch through registry — the SubWorkflow node's per-node
    // workflow_ref reaches the runtime via context.subworkflow_ref.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn dispatch_through_registry_threads_workflow_ref_via_context() {
        use crate::dispatch;
        use onsager_substrate::workflow::Node;

        // Register one runtime instance with a placeholder ref but a
        // capturing runner. Dispatch a node that points at a
        // *different* ref via its substrate executor; the context
        // ref read off the node must win.
        #[derive(Debug, Default)]
        struct CapturingRunner {
            seen: std::sync::Mutex<Option<WorkflowId>>,
        }
        #[async_trait]
        impl SubWorkflowRunner for CapturingRunner {
            async fn run(
                &self,
                workflow_ref: WorkflowId,
                _inputs: Vec<(ArtifactId, Artifact)>,
            ) -> Result<Vec<(ArtifactId, Artifact)>, SubWorkflowRunError> {
                *self.seen.lock().unwrap() = Some(workflow_ref);
                Ok(vec![])
            }
        }
        let capturing: Arc<CapturingRunner> = Arc::new(CapturingRunner::default());
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(
            SubWorkflowExecutor::new(WorkflowId::generate()).with_runner(capturing.clone()),
        ));

        let per_node_ref = WorkflowId::generate();
        let node = Node {
            id: NodeId::generate(),
            executor: Box::new(SubWorkflowExecutor::new(per_node_ref)),
            inputs: vec![],
            outputs: vec![],
        };
        // The scheduler is what reads `node.executor.subworkflow_ref()`
        // into the context. Here we replicate that exactly.
        let ctx = ctx_for_with_ref(node.id, vec![], per_node_ref);
        dispatch(&registry, &node, ctx).await.unwrap();
        assert_eq!(*capturing.seen.lock().unwrap(), Some(per_node_ref));
    }

    // -----------------------------------------------------------------
    // Acceptance test: SchedulerSubWorkflowRunner runs a real inner
    // workflow end-to-end and propagates an Uncertain inner output
    // to the outer SubWorkflow node's output.
    //
    // Per ADR 0011 § "Provenance flows through naturally" and the
    // issue's verification bullet:
    //   > Inner workflow's Uncertain output propagates to outer
    //   > workflow's Uncertain output
    // -----------------------------------------------------------------

    /// An executor that registers under `noop` (so the substrate
    /// `NoOpExecutor` nodes resolve to it at dispatch time) but emits
    /// one `Uncertain { source: Agent }` artifact per call. Use as
    /// the inner workflow's only node to simulate an agent producing
    /// uncertain output inside a SubWorkflow.
    #[derive(Debug)]
    struct UncertainEmittingExecutor;

    #[async_trait]
    impl RuntimeExecutor for UncertainEmittingExecutor {
        fn executor_kind(&self) -> &'static str {
            "noop"
        }
        fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        }
        async fn execute(&self, _: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
            let mut art = Artifact::new(Kind::Code, "inner-agent-output", "marvin", "test", vec![]);
            art.provenance = Provenance::Uncertain {
                source: SourceTag::Agent,
            };
            let id = art.artifact_id.clone();
            Ok(ExecutorOutputs::single(id, art))
        }
    }

    /// Minimal in-memory workflow library for the end-to-end test.
    #[derive(Default)]
    struct InMemoryLibrary {
        by_id: HashMap<WorkflowId, onsager_substrate::workflow::Workflow>,
    }

    impl WorkflowLibrary for InMemoryLibrary {
        fn get(&self, id: WorkflowId) -> Option<&onsager_substrate::workflow::Workflow> {
            self.by_id.get(&id)
        }
    }

    fn inner_workflow_emitting_uncertain() -> onsager_substrate::workflow::Workflow {
        use onsager_substrate::ids::EdgeId;
        use onsager_substrate::workflow::{Edge, EdgeRef, Node, OutputSpec, Workflow};

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
                // The workflow's declared provenance is Uncertain;
                // the producer emits the same. Static validation of
                // the inner workflow against `()` would pass.
                provenance: Provenance::Uncertain {
                    source: SourceTag::Agent,
                },
            }],
        }
    }

    #[tokio::test]
    async fn scheduler_runner_propagates_uncertain_inner_output_to_outer() {
        // Build the inner workflow + library.
        let inner = inner_workflow_emitting_uncertain();
        let inner_id = WorkflowId::generate();
        let mut library = InMemoryLibrary::default();
        library.by_id.insert(inner_id, inner);

        // Build a registry with the uncertain-emitting executor under
        // the `noop` kind, so the inner workflow's NoOp-substrate
        // node dispatches to it.
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(UncertainEmittingExecutor));

        let library: Arc<dyn WorkflowLibrary + Send + Sync> = Arc::new(library);
        let registry = Arc::new(registry);
        let spine: Arc<dyn SpineClient> = Arc::new(MockSpine::default());
        let runner = Arc::new(SchedulerSubWorkflowRunner::with_in_memory_store(
            Arc::clone(&library),
            Arc::clone(&registry),
            Arc::clone(&spine),
        ));

        let exec = SubWorkflowExecutor::new(inner_id).with_runner(runner);
        let outputs = exec
            .execute(ctx_for_with_ref(NodeId::generate(), vec![], inner_id))
            .await
            .expect("inner run succeeds");

        // The SubWorkflow node's output is the inner workflow's exit
        // artifact, complete with its Uncertain provenance.
        assert_eq!(outputs.artifacts.len(), 1);
        let (_id, art) = &outputs.artifacts[0];
        assert!(
            art.provenance.is_uncertain(),
            "expected Uncertain provenance, got {:?}",
            art.provenance,
        );
        assert_eq!(art.provenance.source(), SourceTag::Agent);
    }

    #[tokio::test]
    async fn scheduler_runner_returns_error_when_workflow_ref_unresolved() {
        let library: Arc<dyn WorkflowLibrary + Send + Sync> = Arc::new(InMemoryLibrary::default()); // empty
        let registry = Arc::new(ExecutorRegistry::new());
        let spine: Arc<dyn SpineClient> = Arc::new(MockSpine::default());
        let runner = SchedulerSubWorkflowRunner::with_in_memory_store(library, registry, spine);

        let err = runner
            .run(WorkflowId::generate(), vec![])
            .await
            .expect_err("missing workflow must error");
        assert!(
            err.to_string().contains("not registered in library"),
            "{err}",
        );
    }

    #[tokio::test]
    async fn scheduler_runner_rejects_more_inputs_than_entry_edges() {
        let inner = inner_workflow_emitting_uncertain(); // 0 entry edges
        let inner_id = WorkflowId::generate();
        let mut library = InMemoryLibrary::default();
        library.by_id.insert(inner_id, inner);

        let registry = Arc::new(ExecutorRegistry::new());
        let spine: Arc<dyn SpineClient> = Arc::new(MockSpine::default());
        let runner =
            SchedulerSubWorkflowRunner::with_in_memory_store(Arc::new(library), registry, spine);

        let bogus_input = (ArtifactId::new("bogus"), deterministic_artifact());
        let err = runner
            .run(inner_id, vec![bogus_input])
            .await
            .expect_err("more inputs than entry edges is an error");
        assert!(err.to_string().contains("entry edge"), "{err}");
    }
}
