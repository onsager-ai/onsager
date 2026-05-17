//! Crawlab brownfield fixture — capstone end-to-end test for the 0.2
//! substrate.
//!
//! MIG-03 (#365): re-fixture the Crawlab brownfield scenario on the
//! new substrate APIs. The historical Crawlab fixture did not survive
//! the 0.1 → 0.2 rewrite, so per the issue's "Notes" clause this is a
//! fresh fixture with equivalent complexity — onboard an external
//! brownfield codebase (Crawlab), analyze it with an agent, verify
//! the analysis, and publish a downstream report. The scenario
//! exercises every facility the 0.2 substrate needs to demonstrate
//! it is practically usable:
//!
//! - multi-spec `SpecPlan` (two specs, two different kinds)
//! - cross-kind dependency (`ingest → publish`)
//! - the Plan Compiler (SUB-05, #352) wiring the two subgraphs
//! - the substrate scheduler (RUN-01, #381) running them to terminal
//! - all three production executors (Script/Agent/Verify — EXE-02,
//!   EXE-03, EXE-04)
//! - provenance flow through a Verify node: `Uncertain(Agent)` →
//!   `Deterministic(Composed)`, satisfying a `requires_deterministic`
//!   exit edge
//! - a fixture-local Observer that subscribes to scheduler spine
//!   events and emits an `Insight` summarizing the run (the full
//!   `onsager-observers` crate from OBS-01 / #361 hasn't landed yet;
//!   per ADR 0013 § "Decision" an observer is whatever subscribes to
//!   spine events and emits typed audit outputs without mutating
//!   state, which this minimal in-fixture observer satisfies)
//!
//! The point is the scenario coverage, not the historical naming.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, Provenance, SourceTag};
use onsager_nodes::{
    AgentExecutor, ExecutorRegistry, InMemoryPlanStore, NodeState, PlanId, PlanStore, Scheduler,
    ScriptExecutor, SpineClient, SpineError, StubAgentRunner, VerifyExecutor,
};
use onsager_nodes::{Check, FailPolicy};
use onsager_substrate::NodeId;
use onsager_substrate::compiler::compile;
use onsager_substrate::ids::{EdgeId, WorkflowId};
use onsager_substrate::library::WorkflowLibrary;
use onsager_substrate::spec_plan::{SpecDep, SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::{Edge, EdgeRef, EntrySpec, Node, OutputSpec, Workflow};

// ---------------------------------------------------------------------------
// Fixture-local Observer + Insight.
//
// `onsager-observers` (OBS-01, #361) and the production `Insight` type
// (which will live in that crate) have not landed yet. ADR 0013 fixes
// the *role* — subscribe to spine events, emit typed audit outputs,
// never mutate workflow state — and that is exactly what this minimal
// type set demonstrates. When OBS-01 lands, the fixture will swap to
// the real trait without changing the assertions.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum InsightKind {
    /// A successful run completed end-to-end.
    Win,
}

#[derive(Debug, Clone)]
struct Insight {
    kind: InsightKind,
    observation: String,
    evidence_event_kinds: Vec<String>,
}

/// What a fixture observer reacts to. Returning `Some(Insight)` is the
/// observer's structured audit output for the event; `None` means the
/// event was uninteresting.
trait FixtureObserver: Send + Sync + std::fmt::Debug {
    fn subscribes_to(&self, kind: &str) -> bool;
    fn on_event(&mut self, kind: &str, payload: &serde_json::Value) -> Option<Insight>;
}

/// Observer that counts `node.completed` events on a single plan and
/// emits one `Win` insight once a threshold is reached.
///
/// Mirrors the shape an `onsager-observers` Observer will take (per
/// ADR 0013): subscriptions are pattern-based, output is one of the
/// substrate-recognized audit types, and the observer cannot mutate
/// scheduler state — it only reads events flying past on the spine.
#[derive(Debug)]
struct RunCompletionObserver {
    target_completions: usize,
    seen_completions: usize,
    fired: bool,
    evidence_event_kinds: Vec<String>,
}

impl RunCompletionObserver {
    fn new(target_completions: usize) -> Self {
        Self {
            target_completions,
            seen_completions: 0,
            fired: false,
            evidence_event_kinds: Vec::new(),
        }
    }
}

impl FixtureObserver for RunCompletionObserver {
    fn subscribes_to(&self, kind: &str) -> bool {
        // Glob-equivalent of `plan.*` + `node.*`: every scheduler-side
        // lifecycle event is in scope, mirroring the wildcard
        // subscription model OBS-01 will offer.
        kind.starts_with("plan.") || kind.starts_with("node.")
    }

    fn on_event(&mut self, kind: &str, _payload: &serde_json::Value) -> Option<Insight> {
        self.evidence_event_kinds.push(kind.to_string());
        if kind == "node.completed" {
            self.seen_completions += 1;
            if !self.fired && self.seen_completions >= self.target_completions {
                self.fired = true;
                return Some(Insight {
                    kind: InsightKind::Win,
                    observation: format!(
                        "run completed {} nodes successfully (Crawlab brownfield pipeline)",
                        self.seen_completions,
                    ),
                    evidence_event_kinds: self.evidence_event_kinds.clone(),
                });
            }
        }
        None
    }
}

/// `SpineClient` that forwards every emit to a set of `FixtureObserver`s
/// and collects any `Insight`s they produce. Per ADR 0013 § "Observer
/// properties": the observer runs *off* the workflow path — emits
/// fire-and-forget into the spine, the observer reads asynchronously.
/// In a single-test process that's a synchronous fan-out under a mutex;
/// in production OBS-01 will spawn each observer in its own task.
#[derive(Debug)]
struct ObservingSpine {
    observers: Mutex<Vec<Box<dyn FixtureObserver>>>,
    insights: Mutex<Vec<Insight>>,
    emitted: Mutex<Vec<(String, serde_json::Value)>>,
}

impl ObservingSpine {
    fn new(observers: Vec<Box<dyn FixtureObserver>>) -> Self {
        Self {
            observers: Mutex::new(observers),
            insights: Mutex::new(Vec::new()),
            emitted: Mutex::new(Vec::new()),
        }
    }

    fn insights(&self) -> Vec<Insight> {
        self.insights.lock().unwrap().clone()
    }

    fn emitted_kinds(&self) -> Vec<String> {
        self.emitted
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }
}

#[async_trait]
impl SpineClient for ObservingSpine {
    async fn emit(&self, kind: &str, payload: serde_json::Value) -> Result<(), SpineError> {
        self.emitted
            .lock()
            .unwrap()
            .push((kind.to_string(), payload.clone()));
        let mut new_insights = Vec::new();
        {
            let mut observers = self.observers.lock().unwrap();
            for observer in observers.iter_mut() {
                if observer.subscribes_to(kind)
                    && let Some(insight) = observer.on_event(kind, &payload)
                {
                    new_insights.push(insight);
                }
            }
        }
        if !new_insights.is_empty() {
            self.insights.lock().unwrap().extend(new_insights);
        }
        Ok(())
    }

    async fn read_artifact(&self, _: &ArtifactId) -> Result<Option<Artifact>, SpineError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Workflow Library — `crawlab-ingest` and `crawlab-publish`.
// ---------------------------------------------------------------------------

struct FixtureLibrary {
    by_id: HashMap<WorkflowId, Workflow>,
    by_kind: HashMap<String, WorkflowId>,
}

impl FixtureLibrary {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_kind: HashMap::new(),
        }
    }

    fn register(&mut self, kind: &str, workflow: Workflow) {
        let id = WorkflowId::generate();
        self.by_id.insert(id, workflow);
        self.by_kind.insert(kind.to_string(), id);
    }
}

impl WorkflowLibrary for FixtureLibrary {
    fn get(&self, id: WorkflowId) -> Option<&Workflow> {
        self.by_id.get(&id)
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        self.by_kind.get(kind).and_then(|id| self.by_id.get(id))
    }
}

/// The "ingest" workflow — Script ▸ Agent ▸ Verify.
///
/// Simulates onboarding the Crawlab repo: a deterministic script
/// produces a repo inventory, an agent analyses it (Uncertain by ADR
/// 0010), a Verify node certifies the analysis. The exit edge is
/// declared `requires_deterministic`, so without the Verify upgrade
/// the workflow would fail invariant 1.
fn crawlab_ingest_workflow() -> Workflow {
    let inv_to_agent = EdgeId::generate();
    let agent_to_verify = EdgeId::generate();
    let verify_exit = EdgeId::generate();

    let inventory = Node {
        id: NodeId::generate(),
        executor: Box::new(ScriptExecutor::new([
            "sh",
            "-c",
            "echo 'crawlab/main.go\\ncrawlab/web/index.html\\ncrawlab/README.md'",
        ])),
        inputs: vec![],
        outputs: vec![EdgeRef::new(inv_to_agent)],
    };
    let analyse = Node {
        id: NodeId::generate(),
        executor: Box::new(
            AgentExecutor::new(
                "claude-sonnet-4-6",
                "you are a code-review agent — analyse the Crawlab inventory",
            )
            .with_runner(Arc::new(StubAgentRunner::new(
                "analysis: README is light, web/ lacks tests, main.go has a stale TODO",
            ))),
        ),
        inputs: vec![EdgeRef::new(inv_to_agent)],
        outputs: vec![EdgeRef::new(agent_to_verify)],
    };
    let verify = Node {
        id: NodeId::generate(),
        executor: Box::new(VerifyExecutor::with_checks(
            vec![
                Check::TestRun {
                    name: "crawlab-smoke".into(),
                    must_pass: true,
                },
                Check::Lint {
                    name: "crawlab-lint".into(),
                    must_pass: true,
                },
            ],
            FailPolicy::Escalate,
        )),
        inputs: vec![EdgeRef::new(agent_to_verify)],
        outputs: vec![EdgeRef::new(verify_exit)],
    };

    Workflow {
        nodes: vec![inventory, analyse, verify],
        edges: vec![
            Edge {
                id: inv_to_agent,
                artifact_id: ArtifactId::new("inventory"),
                requires_deterministic: false,
            },
            Edge {
                id: agent_to_verify,
                artifact_id: ArtifactId::new("analysis"),
                // Agent emits Uncertain — this edge cannot demand
                // Deterministic. The next edge (verify_exit) does.
                requires_deterministic: false,
            },
            Edge {
                id: verify_exit,
                artifact_id: ArtifactId::new("verified"),
                // The capstone provenance check: Verify is the *only*
                // executor allowed to satisfy this against an Uncertain
                // upstream (invariant 1 + ADR 0010).
                requires_deterministic: true,
            },
        ],
        entry_specs: vec![],
        output_specs: vec![OutputSpec {
            edge_id: verify_exit,
            provenance: Provenance::Deterministic {
                source: SourceTag::Composed,
            },
        }],
    }
}

/// The "publish" workflow — single Script node.
///
/// Consumes the verified analysis off its entry edge and emits a
/// publish artifact. Declares an entry slot so the Plan Compiler can
/// rewire it to the upstream ingest spec's exit per ADR 0017.
fn crawlab_publish_workflow() -> Workflow {
    let entry = EdgeId::generate();
    let publish_out = EdgeId::generate();

    let publish = Node {
        id: NodeId::generate(),
        executor: Box::new(ScriptExecutor::new([
            "sh",
            "-c",
            "echo 'published: crawlab onboarding report v1'",
        ])),
        inputs: vec![EdgeRef::new(entry)],
        outputs: vec![EdgeRef::new(publish_out)],
    };

    Workflow {
        nodes: vec![publish],
        edges: vec![
            Edge {
                id: entry,
                artifact_id: ArtifactId::new("publish_input"),
                // Publish only consumes verified analyses. Cross-spec
                // wiring keeps this edge attached to ingest's
                // deterministic exit after compile.
                requires_deterministic: true,
            },
            Edge {
                id: publish_out,
                artifact_id: ArtifactId::new("published"),
                requires_deterministic: false,
            },
        ],
        entry_specs: vec![EntrySpec { edge_id: entry }],
        output_specs: vec![OutputSpec {
            edge_id: publish_out,
            provenance: Provenance::Deterministic {
                source: SourceTag::Script,
            },
        }],
    }
}

fn build_library() -> FixtureLibrary {
    let mut lib = FixtureLibrary::new();
    lib.register("crawlab-ingest", crawlab_ingest_workflow());
    lib.register("crawlab-publish", crawlab_publish_workflow());
    lib
}

fn build_spec_plan() -> SpecPlan {
    SpecPlan {
        specs: vec![
            SpecRef {
                id: SpecId::new("crawlab"),
                kind: "crawlab-ingest".into(),
                inputs: Default::default(),
            },
            SpecRef {
                id: SpecId::new("publish"),
                kind: "crawlab-publish".into(),
                inputs: Default::default(),
            },
        ],
        deps: vec![SpecDep {
            from: SpecId::new("crawlab"),
            to: SpecId::new("publish"),
        }],
    }
}

// ---------------------------------------------------------------------------
// The acceptance test.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn crawlab_brownfield_pipeline_runs_end_to_end() {
    // ---- arrange ---------------------------------------------------------
    let lib = build_library();
    let plan = build_spec_plan();

    // Compile the SpecPlan into an ExecutionPlan. The compiler runs
    // every kernel invariant (ADR 0018); a green compile is the
    // proof that the Verify node satisfies the `requires_deterministic`
    // exit edge despite the Agent upstream being Uncertain.
    let exec_plan = compile(&plan, &lib).expect("Spec Plan compiles cleanly");

    // Two specs × (3 + 1) nodes — minus 0 because no edges collapse
    // across the cross-kind boundary (entry_edge gets rewired but the
    // node count is preserved).
    assert_eq!(
        exec_plan.nodes.len(),
        4,
        "ingest contributes 3 nodes, publish 1 — total 4"
    );

    // The cross-kind dep should have rewired `publish`'s entry edge
    // to ingest's exit edge id, so the publish node's input is now
    // the verify node's output.
    let publish_slot = &exec_plan.spec_index[&SpecId::new("publish")];
    let ingest_slot = &exec_plan.spec_index[&SpecId::new("crawlab")];
    assert_eq!(ingest_slot.exit_edges.len(), 1);
    assert_eq!(publish_slot.entry_edges.len(), 1);
    assert_eq!(
        publish_slot.entry_edges[0], ingest_slot.exit_edges[0].edge_id,
        "publish's entry should be wired to ingest's exit after compile",
    );

    // Sanity-check that the per-node configuration survives `Workflow::instantiate`'s
    // serde round-trip (the Plan Compiler re-serializes each executor
    // to deep-copy it). Without this, a regression that loses the
    // node-level command / prompt / checks would still let the
    // registry-dispatch path produce green output — silently passing.
    //
    // Today (RUN-01 / #381), the scheduler dispatches by `executor_kind`
    // and runs the *registry* instance instead of the node's; threading
    // per-node config through dispatch is RUN-02 (#360). Until then,
    // the compile-side guarantee that node config round-trips is the
    // best mechanical assertion we can make.
    let kinds: Vec<&str> = exec_plan
        .nodes
        .iter()
        .map(|n| n.executor.executor_kind())
        .collect();
    assert!(
        kinds.contains(&"script") && kinds.contains(&"agent") && kinds.contains(&"verify"),
        "compiled plan should carry all three executor kinds (got: {kinds:?})",
    );
    let script_nodes: Vec<&Node> = exec_plan
        .nodes
        .iter()
        .filter(|n| n.executor.executor_kind() == "script")
        .collect();
    assert_eq!(
        script_nodes.len(),
        2,
        "two Script nodes (inventory + publish) survive compile",
    );
    // Serialize each Script node's executor and check the configured
    // command landed in the compiled plan — proves the per-node config
    // is preserved through `Workflow::instantiate`'s deep copy.
    let scripted_commands: Vec<String> = script_nodes
        .iter()
        .map(|n| serde_json::to_value(&n.executor).unwrap()["command"].to_string())
        .collect();
    assert!(
        scripted_commands
            .iter()
            .any(|c| c.contains("crawlab/main.go")),
        "inventory Script's argv must survive compile (got: {scripted_commands:?})",
    );
    assert!(
        scripted_commands
            .iter()
            .any(|c| c.contains("published: crawlab onboarding report")),
        "publish Script's argv must survive compile (got: {scripted_commands:?})",
    );
    let verify_node = exec_plan
        .nodes
        .iter()
        .find(|n| n.executor.executor_kind() == "verify")
        .expect("verify node lives in the compiled plan");
    let verify_json = serde_json::to_value(&verify_node.executor).unwrap();
    let check_names: Vec<String> = verify_json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        check_names,
        vec!["crawlab-smoke".to_string(), "crawlab-lint".to_string()],
        "Verify's check list must survive compile",
    );

    // ---- act -------------------------------------------------------------
    // Today's `dispatch` (RUN-01) resolves a node to *the registry's*
    // instance for that kind — not to the node's own boxed executor
    // (see `crates/onsager-nodes/src/dispatch.rs`). Threading per-node
    // configuration through dispatch is RUN-02 (#360). The compile-
    // side assertions above prove the node-level config survives
    // instantiation; here we register one canonical handler per kind
    // that the scheduler actually invokes during this run.
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(ScriptExecutor::new([
        "sh",
        "-c",
        "echo 'crawlab brownfield pipeline output'",
    ])));
    registry.register(Arc::new(
        AgentExecutor::new("claude-sonnet-4-6", "")
            .with_runner(Arc::new(StubAgentRunner::new("analysis ok"))),
    ));
    registry.register(Arc::new(VerifyExecutor::with_checks(
        vec![Check::TestRun {
            name: "registry-default".into(),
            must_pass: true,
        }],
        FailPolicy::Escalate,
    )));

    // 4 completion events expected — one per terminal node.
    let observer: Box<dyn FixtureObserver> = Box::new(RunCompletionObserver::new(4));
    let spine = Arc::new(ObservingSpine::new(vec![observer]));

    let store = Arc::new(InMemoryPlanStore::new());
    let scheduler = Scheduler::new(
        Arc::new(registry),
        Arc::clone(&store) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    );

    let plan_id = PlanId::generate();
    scheduler
        .run(&plan_id, &exec_plan)
        .await
        .expect("Crawlab brownfield pipeline runs to completion");

    // ---- assert ----------------------------------------------------------
    // 1) All 4 nodes terminated `Completed`.
    let states = store.node_states(&plan_id).await.unwrap();
    assert_eq!(states.len(), 4, "every node should have a persisted state");
    for state in states.values() {
        assert_eq!(
            *state,
            NodeState::Completed,
            "every node should reach Completed (got {state:?})",
        );
    }

    // 2) Artifacts materialized on every output edge.
    let output_edges: Vec<&Edge> = exec_plan
        .edges
        .iter()
        .filter(|e| {
            exec_plan
                .nodes
                .iter()
                .any(|n| n.outputs.iter().any(|o| o.edge_id == e.id))
        })
        .collect();
    assert!(
        !output_edges.is_empty(),
        "compiled plan should have output edges to materialize",
    );
    let mut by_artifact: HashMap<ArtifactId, Artifact> = HashMap::new();
    for edge in &output_edges {
        let art = store
            .get_artifact(&plan_id, &edge.artifact_id)
            .await
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "expected artifact at {} (edge {})",
                    edge.artifact_id, edge.id,
                )
            });
        assert_eq!(
            &art.artifact_id, &edge.artifact_id,
            "persisted artifact key and body id must agree",
        );
        by_artifact.insert(edge.artifact_id.clone(), art);
    }

    // 3) Provenance correct on each output:
    //    - inventory (Script with no inputs)  → Deterministic { Script }
    //    - analysis (Agent — Uncertain colours its output)
    //                                        → Uncertain { Agent }
    //    - verified (Verify — composes inputs)
    //                                        → Deterministic { Composed }
    //    - published (Script consuming verified — kernel invariant 2
    //      lets a non-Verify executor stay deterministic when its
    //      upstream is itself deterministic, which Verify guarantees)
    //                                        → Deterministic { Script }
    let inventory = artifact_for(&by_artifact, "crawlab", "inventory");
    let analysis = artifact_for(&by_artifact, "crawlab", "analysis");
    let verified = artifact_for(&by_artifact, "crawlab", "verified");
    let published = artifact_for(&by_artifact, "publish", "published");

    assert_eq!(
        inventory.provenance,
        Provenance::Deterministic {
            source: SourceTag::Script
        },
        "Script ingest emits Deterministic(Script)",
    );
    assert_eq!(
        analysis.provenance,
        Provenance::Uncertain {
            source: SourceTag::Agent
        },
        "Agent always colours its output Uncertain(Agent) — ADR 0010 / EXE-03",
    );
    // The capstone assertion — Verify is the *only* node permitted to
    // promote Uncertain upstream to Deterministic downstream.
    assert_eq!(
        verified.provenance,
        Provenance::Deterministic {
            source: SourceTag::Composed
        },
        "Verify upgrades Uncertain(Agent) to Deterministic(Composed) (ADR 0010)",
    );
    assert!(
        !verified.provenance.is_uncertain(),
        "Verify output must clear is_uncertain() so requires_deterministic edges accept it",
    );
    assert_eq!(
        published.provenance,
        Provenance::Deterministic {
            source: SourceTag::Script
        },
        "Downstream Script reads Verify's deterministic exit and stays deterministic",
    );

    // 4) The Observer subscribed to scheduler events emitted at least
    //    one Insight. Confirms the audit-loop primitive ADR 0013
    //    commits to: a non-blocking subscriber to spine events
    //    producing typed audit output.
    let insights = spine.insights();
    assert!(
        !insights.is_empty(),
        "observer should have emitted at least one Insight (audit loop is wired)",
    );
    let win = insights
        .iter()
        .find(|i| i.kind == InsightKind::Win)
        .expect("a Win insight should fire once the pipeline completes");
    assert!(
        win.observation.contains("Crawlab"),
        "insight should name the pipeline (Crawlab) in its observation, got: {}",
        win.observation,
    );
    // Sanity: the evidence references the scheduler's actual event
    // stream — not invented strings.
    let emitted = spine.emitted_kinds();
    assert!(
        win.evidence_event_kinds.iter().all(|k| emitted.contains(k)),
        "insight evidence must reference events the scheduler actually emitted",
    );
    assert!(
        emitted.iter().filter(|k| *k == "node.completed").count() == 4,
        "the four nodes' completion events should all be on the spine",
    );
}

/// Helper: look up the artifact persisted under a given spec namespace
/// and edge `artifact_id`. The Plan Compiler namespaces every edge's
/// `artifact_id` by prefixing `"<spec_id>:"` (per ADR 0017's
/// `Workflow::instantiate` rule) so two specs of the same kind don't
/// collide on invariant 5.
fn artifact_for<'a>(
    by_artifact: &'a HashMap<ArtifactId, Artifact>,
    spec: &str,
    name: &str,
) -> &'a Artifact {
    let id = ArtifactId::new(format!("{spec}:{name}"));
    by_artifact
        .get(&id)
        .unwrap_or_else(|| panic!("expected materialized artifact {id} in store"))
}
