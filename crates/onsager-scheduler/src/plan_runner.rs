//! [`PlanRunner`] — the one run entry both front doors share (B2,
//! #500, ADR 0023 decision 1).
//!
//! Before this, the single-spec `trigger.fired` flow
//! ([`crate::bridge`]) was the only deployed `Scheduler::run` caller,
//! and it hardcoded a one-element [`SpecPlan`] over an ephemeral store.
//! `PlanRunner` collapses that into a single entry: compile a
//! [`SpecPlan`] against a workflow-library lookup, then drive
//! [`Scheduler::run`] against the durable [`PlanStore`]. Both the
//! single-spec trigger path and the multi-spec `plan.run_requested`
//! path call it — no parallel run loops.

use std::collections::HashMap;
use std::sync::Arc;

use onsager_nodes::{ExecutorRegistry, PlanId, PlanStore, Scheduler, SchedulerError, SpineClient};
use onsager_substrate::compiler::{CompileError, compile};
use onsager_substrate::ids::WorkflowId;
use onsager_substrate::spec_plan::SpecPlan;
use onsager_substrate::workflow::Workflow;
use onsager_substrate::workflow_library::WorkflowLibrary as PersistedWorkflowLibrary;
use onsager_substrate::{WorkflowLibraryError, WorkflowLookup};
use thiserror::Error;

/// Failure running a [`SpecPlan`] end-to-end.
#[derive(Debug, Error)]
pub enum PlanRunError {
    /// The Spec Plan failed to compile (structural error, missing
    /// kind, kernel invariant violation, …).
    #[error("compile failed: {0}")]
    Compile(#[from] CompileError),

    /// The scheduler aborted the run (a node failed, the plan got
    /// stuck).
    #[error("scheduler failed: {0}")]
    Scheduler(#[from] SchedulerError),
}

/// Compiles a [`SpecPlan`] and runs it through [`Scheduler`] against a
/// durable [`PlanStore`]. Built once at host startup (the store,
/// registry, and spine are process-wide) and reused per run.
#[derive(Clone)]
pub struct PlanRunner {
    registry: Arc<ExecutorRegistry>,
    store: Arc<dyn PlanStore>,
    spine: Arc<dyn SpineClient>,
}

impl std::fmt::Debug for PlanRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanRunner").finish_non_exhaustive()
    }
}

impl PlanRunner {
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

    /// Compile `spec_plan` against `library` and run it under
    /// `plan_id` to a terminal state. Restart-safe via the durable
    /// store: re-running the same `plan_id` resumes rather than
    /// restarting (see [`Scheduler::run`]).
    ///
    /// `library` is generic + `Sync` (rather than a bare `&dyn
    /// WorkflowLookup`) so the produced future is `Send` — the spine
    /// listener loop requires a `Send` handler, and a `&dyn` trait
    /// object defaults to non-`Sync` even though every concrete lookup
    /// here is `Send + Sync` (the `Executor` trait is `Send + Sync`).
    pub async fn run<L: WorkflowLookup + Sync>(
        &self,
        plan_id: &PlanId,
        spec_plan: &SpecPlan,
        library: &L,
    ) -> Result<(), PlanRunError> {
        let exec_plan = compile(spec_plan, library)?;
        let scheduler = Scheduler::new(
            Arc::clone(&self.registry),
            Arc::clone(&self.store),
            Arc::clone(&self.spine),
        );
        scheduler.run(plan_id, &exec_plan).await?;
        Ok(())
    }
}

/// An in-memory snapshot of the workflow library covering exactly the
/// kinds a [`SpecPlan`] references, built by resolving each spec's
/// `kind` through the persisted [`PersistedWorkflowLibrary`] once.
///
/// The Plan Compiler needs a synchronous `&dyn WorkflowLookup`, but
/// the persisted library is async (one query per `latest(kind)`); this
/// snapshot bridges the two — the same shape portal's `compile_dry_run`
/// uses on its side. SubWorkflow refs ([`WorkflowLookup::get`] by
/// `WorkflowId`) are not resolved here (v1 — same limitation as the
/// single-spec trigger path); a plan whose workflow embeds a
/// SubWorkflow executor fails invariant 4 at compile, loudly.
#[derive(Debug, Default)]
pub struct SchedulerLibrarySnapshot {
    by_kind: HashMap<String, Workflow>,
}

impl SchedulerLibrarySnapshot {
    /// Resolve every distinct `kind` in `spec_plan` against the
    /// persisted library, pinning each kind to the version recorded in
    /// `kind_versions` (#511). Kinds with no registered workflow are
    /// simply absent from the snapshot — the compiler then surfaces
    /// `CompileError::MissingKind`, which is the correct hard failure
    /// per ADR 0017.
    ///
    /// `kind_versions` is the `{kind → version}` snapshot persisted at
    /// registration ([`resolve_kind_versions`]). A kind present in the
    /// map resolves via [`lookup_versioned`](PersistedWorkflowLibrary::lookup_versioned)
    /// so a plan recovered after a restart recompiles against the
    /// version it began on. A kind **absent** from the map (legacy rows
    /// registered before #511, whose `kind_versions` is `{}`) or one
    /// whose pinned version was retired falls back to
    /// [`lookup`](PersistedWorkflowLibrary::lookup) (latest), logging a
    /// warning so the divergence is observable rather than silent.
    pub async fn build(
        spec_plan: &SpecPlan,
        library: &PersistedWorkflowLibrary,
        kind_versions: &HashMap<String, i32>,
    ) -> Result<Self, WorkflowLibraryError> {
        let mut by_kind = HashMap::new();
        for spec in &spec_plan.specs {
            if by_kind.contains_key(&spec.kind) {
                continue;
            }
            let resolved = match kind_versions.get(&spec.kind) {
                Some(&version) => match library.lookup_versioned(&spec.kind, version).await? {
                    Some(workflow) => Some(workflow),
                    None => {
                        tracing::warn!(
                            kind = %spec.kind,
                            pinned_version = version,
                            "pinned workflow version is absent or retired; \
                             recovering against latest() instead",
                        );
                        library.lookup(&spec.kind).await?
                    }
                },
                None => library.lookup(&spec.kind).await?,
            };
            if let Some(workflow) = resolved {
                by_kind.insert(spec.kind.clone(), workflow);
            }
        }
        Ok(Self { by_kind })
    }
}

/// Snapshot the `{kind → version}` map a [`SpecPlan`] compiles against:
/// the latest registered version for every distinct kind it
/// references. Persisted by `register_plan` (#511) so a later recovery
/// recompiles against these exact versions rather than whatever is
/// latest at restart. Kinds with no registered workflow are omitted —
/// the recovering `build` then falls through to its (also empty)
/// `latest()` lookup and the compiler surfaces the missing kind.
pub async fn resolve_kind_versions(
    spec_plan: &SpecPlan,
    library: &PersistedWorkflowLibrary,
) -> Result<HashMap<String, i32>, WorkflowLibraryError> {
    let mut versions = HashMap::new();
    for spec in &spec_plan.specs {
        if versions.contains_key(&spec.kind) {
            continue;
        }
        match library.latest_version(&spec.kind).await? {
            Some(version) => {
                versions.insert(spec.kind.clone(), version);
            }
            None => {
                // No registered version to pin — the compile will
                // surface `MissingKind` for this spec. Log it so the
                // unregistered kind is visible at registration time, not
                // only when the run later fails to compile.
                tracing::warn!(
                    kind = %spec.kind,
                    "no registered workflow version for kind at plan registration; \
                     it will not be pinned",
                );
            }
        }
    }
    Ok(versions)
}

impl WorkflowLookup for SchedulerLibrarySnapshot {
    fn get(&self, _id: WorkflowId) -> Option<&Workflow> {
        None
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        self.by_kind.get(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use onsager_artifact::{Artifact, ArtifactId, Kind, NodeId, Provenance};
    use onsager_nodes::{
        Executor, ExecutorContext, ExecutorError, ExecutorOutputs, ExecutorRegistry,
        InMemoryPlanStore, SpineError,
    };
    use onsager_substrate::executor::NoOpExecutor;
    use onsager_substrate::ids::EdgeId;
    use onsager_substrate::spec_plan::{SpecDep, SpecId, SpecRef};
    use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec};
    use std::sync::Mutex;

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

    /// Runtime executor (kind `noop`) that records every input
    /// artifact id its context received and produces one output
    /// artifact per declared output edge. Lets a test prove the
    /// downstream spec's node observed the upstream spec's output —
    /// i.e. the dep was wired and ordering was honored.
    #[derive(Debug, Default)]
    struct RecordingEcho {
        seen_inputs: Mutex<Vec<ArtifactId>>,
    }

    #[async_trait]
    impl Executor for RecordingEcho {
        fn executor_kind(&self) -> &'static str {
            "noop"
        }
        fn declared_provenance(&self, _: &[Provenance]) -> Provenance {
            Provenance::external_deterministic()
        }
        async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
            {
                let mut seen = self.seen_inputs.lock().unwrap();
                for (id, _) in &ctx.inputs {
                    seen.push(id.clone());
                }
            }
            let art = Artifact::new(Kind::Document, "echo", "marvin", "scheduler", vec![]);
            let id = art.artifact_id.clone();
            Ok(ExecutorOutputs::single(id, art))
        }
    }

    /// Workflow with one node: (entry) -> [noop] -> (exit). Used for
    /// both specs so the only difference is the dep wiring.
    fn passthrough_workflow() -> Workflow {
        let entry = EdgeId::generate();
        let exit = EdgeId::generate();
        Workflow {
            nodes: vec![Node {
                id: NodeId::generate(),
                executor: Box::new(NoOpExecutor),
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

    struct TwoKindLibrary {
        up: Workflow,
        down: Workflow,
    }

    impl WorkflowLookup for TwoKindLibrary {
        fn get(&self, _: WorkflowId) -> Option<&Workflow> {
            None
        }
        fn by_kind(&self, kind: &str) -> Option<&Workflow> {
            match kind {
                "up" => Some(&self.up),
                "down" => Some(&self.down),
                _ => None,
            }
        }
    }

    fn build_runner() -> (PlanRunner, std::sync::Arc<RecordingEcho>) {
        let recorder = std::sync::Arc::new(RecordingEcho::default());
        let mut registry = ExecutorRegistry::new();
        registry.register(std::sync::Arc::clone(&recorder) as std::sync::Arc<dyn Executor>);
        let store: std::sync::Arc<dyn PlanStore> = std::sync::Arc::new(InMemoryPlanStore::new());
        let runner = PlanRunner::new(
            std::sync::Arc::new(registry),
            store,
            std::sync::Arc::new(NoopSpine) as std::sync::Arc<dyn SpineClient>,
        );
        (runner, recorder)
    }

    /// A persisted 2-spec plan with one dep compiles and runs both
    /// specs in dependency order: the downstream node observes the
    /// upstream node's output artifact, persisted under the rewired
    /// exit edge.
    ///
    /// Mutation check: drop the `deps` edge (see
    /// `two_spec_plan_without_dep_sees_no_upstream_output`) and the
    /// downstream node sees no inputs — this assertion fails.
    #[tokio::test]
    async fn two_spec_plan_runs_in_dependency_order() {
        let (runner, recorder) = build_runner();
        let lib = TwoKindLibrary {
            up: passthrough_workflow(),
            down: passthrough_workflow(),
        };
        let plan = SpecPlan {
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
        };
        let plan_id = PlanId::generate();
        runner.run(&plan_id, &plan, &lib).await.expect("run ok");

        // s1's exit edge artifact id is `s1:art_out` (namespaced by
        // instantiate); the dep rewires s2's entry to consume it, so
        // the downstream node must have seen it.
        let seen = recorder.seen_inputs.lock().unwrap();
        assert!(
            seen.iter().any(|a| a.as_str() == "s1:art_out"),
            "downstream node should have consumed the upstream output, saw: {seen:?}",
        );
    }

    #[tokio::test]
    async fn two_spec_plan_without_dep_sees_no_upstream_output() {
        // Same plan, no dep — the mutation. Each spec's entry edge
        // stays its own (no producer), so the downstream node sees no
        // input and the dependency-order assertion above would fail.
        let (runner, recorder) = build_runner();
        let lib = TwoKindLibrary {
            up: passthrough_workflow(),
            down: passthrough_workflow(),
        };
        let plan = SpecPlan {
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
            deps: vec![],
        };
        let plan_id = PlanId::generate();
        runner.run(&plan_id, &plan, &lib).await.expect("run ok");

        let seen = recorder.seen_inputs.lock().unwrap();
        assert!(
            !seen.iter().any(|a| a.as_str() == "s1:art_out"),
            "without the dep, the downstream node must not see s1's output",
        );
    }
}
