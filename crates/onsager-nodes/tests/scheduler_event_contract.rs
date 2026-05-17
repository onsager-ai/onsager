//! RUN-02 (#360) verification: an upstream → Agent → Verify run emits
//! the complete substrate event contract on the spine.
//!
//! From the issue's verification list:
//!
//! > A Workflow run (Script → Agent → Verify) emits all expected event
//! > types in spine.
//!
//! `ScriptExecutor` does not implement the substrate `Executor` trait
//! today — that's a follow-up that needs serde + typetag wiring on
//! `ScriptExecutor`. Until then the upstream slot in this test is
//! `NoOpExecutor`, which exercises the same scheduler-side lifecycle
//! emission contract (`node.started` → `node.completed`) every Script
//! node would; the executor-specific events `script.*` are not part
//! of the #360 contract anyway, so the substitution does not weaken
//! the assertion.
//!
//! Expected events per node lifecycle:
//!
//! - Every node: one `node.started`, then one of
//!   `node.completed` / `node.failed`.
//! - Agent node only: one `agent.session_started`, then one of
//!   `agent.session_completed` / `agent.session_failed`.
//! - Verify node only: one `synodic.verdict` (pass or fail).
//!
//! `node.awaiting_human` / `node.human_approved` / `node.human_rejected`
//! are the Human executor's contract (#357); not exercised here.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use onsager_artifact::{Artifact, ArtifactId, NodeId};
use onsager_nodes::{
    AgentExecutor, EVENT_NODE_COMPLETED, EVENT_NODE_FAILED, EVENT_NODE_STARTED, ExecutorRegistry,
    InMemoryPlanStore, PlanId, PlanStore, Scheduler, SpineClient, SpineError, StubAgentRunner,
    VerifyExecutor,
    verify::{Check, FailPolicy},
};
use onsager_substrate::compiler::ExecutionPlan;
use onsager_substrate::events as se;
use onsager_substrate::executor::NoOpExecutor;
use onsager_substrate::ids::EdgeId;
use onsager_substrate::workflow::{Edge, EdgeRef, Node};
use serde_json::Value;

#[derive(Debug, Default)]
struct CapturingSpine {
    emitted: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl SpineClient for CapturingSpine {
    async fn emit(&self, kind: &str, payload: Value) -> Result<(), SpineError> {
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

/// Linear plan: Script → Agent → Verify, threaded through three edges.
/// Each node gets a runtime executor wired via the
/// `ExecutorRegistry` so dispatch resolves by `executor_kind()`.
fn script_agent_verify_plan() -> (ExecutionPlan, NodeId, NodeId, NodeId) {
    let e_script_to_agent = EdgeId::generate();
    let e_agent_to_verify = EdgeId::generate();
    let e_verify_out = EdgeId::generate();

    let script_node_id = NodeId::generate();
    let agent_node_id = NodeId::generate();
    let verify_node_id = NodeId::generate();

    let script = Node {
        id: script_node_id,
        executor: Box::new(NoOpExecutor),
        inputs: vec![],
        outputs: vec![EdgeRef::new(e_script_to_agent)],
    };
    let agent = Node {
        id: agent_node_id,
        executor: Box::new(AgentExecutor::new("claude-sonnet-4-6", "you are helpful")),
        inputs: vec![EdgeRef::new(e_script_to_agent)],
        outputs: vec![EdgeRef::new(e_agent_to_verify)],
    };
    let verify = Node {
        id: verify_node_id,
        executor: Box::new(VerifyExecutor::with_checks(
            vec![Check::TestRun {
                name: "cargo_test".into(),
                must_pass: true,
            }],
            FailPolicy::Escalate,
        )),
        inputs: vec![EdgeRef::new(e_agent_to_verify)],
        outputs: vec![EdgeRef::new(e_verify_out)],
    };

    let plan = ExecutionPlan {
        nodes: vec![script, agent, verify],
        edges: vec![
            Edge {
                id: e_script_to_agent,
                artifact_id: ArtifactId::new("script_out"),
                requires_deterministic: false,
            },
            Edge {
                id: e_agent_to_verify,
                artifact_id: ArtifactId::new("agent_out"),
                requires_deterministic: false,
            },
            Edge {
                id: e_verify_out,
                artifact_id: ArtifactId::new("verify_out"),
                requires_deterministic: false,
            },
        ],
        spec_index: Default::default(),
    };
    (plan, script_node_id, agent_node_id, verify_node_id)
}

fn registry_with_agent_stub() -> ExecutorRegistry {
    // `with_noop` registers a runtime `NoOpExecutor` under "noop" so
    // the upstream substitute dispatches. Verify is registered under
    // "verify"; the agent under "agent" with a stub runner so the
    // test never reaches the network.
    let mut reg = ExecutorRegistry::with_noop();
    reg.register(Arc::new(VerifyExecutor::default()));
    reg.register(Arc::new(
        AgentExecutor::new("claude-sonnet-4-6", "you are helpful")
            .with_runner(Arc::new(StubAgentRunner::new("agent said hi"))),
    ));
    reg
}

#[tokio::test]
async fn script_agent_verify_emits_full_substrate_contract() {
    let (plan, script_id, agent_id, verify_id) = script_agent_verify_plan();
    let registry = Arc::new(registry_with_agent_stub());
    let store = Arc::new(InMemoryPlanStore::new());
    let spine = Arc::new(CapturingSpine::default());
    let scheduler = Scheduler::new(
        registry,
        Arc::clone(&store) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    );
    let plan_id = PlanId::generate();

    scheduler.run(&plan_id, &plan).await.unwrap();

    let emitted = spine.emitted.lock().unwrap().clone();
    let kinds: Vec<&str> = emitted.iter().map(|(k, _)| k.as_str()).collect();

    // -- Scheduler-side lifecycle: 3 starts + 3 completions, 0 failures.
    assert_eq!(
        kinds.iter().filter(|k| **k == EVENT_NODE_STARTED).count(),
        3,
        "expected node.started per node, got: {kinds:?}",
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == EVENT_NODE_COMPLETED).count(),
        3,
        "expected node.completed per node, got: {kinds:?}",
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == EVENT_NODE_FAILED).count(),
        0,
        "no node should have failed in the happy-path run",
    );

    // -- Agent executor: session-started + session-completed.
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == se::KIND_AGENT_SESSION_STARTED)
            .count(),
        1,
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == se::KIND_AGENT_SESSION_COMPLETED)
            .count(),
        1,
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == se::KIND_AGENT_SESSION_FAILED)
            .count(),
        0,
    );

    // -- Verify executor: one synodic.verdict, passed=true.
    let verdicts: Vec<&Value> = emitted
        .iter()
        .filter(|(k, _)| k == se::KIND_SYNODIC_VERDICT)
        .map(|(_, p)| p)
        .collect();
    assert_eq!(verdicts.len(), 1, "expected one synodic.verdict emit");
    assert_eq!(verdicts[0]["passed"], true);
    assert!(verdicts[0]["check_results"].is_array());

    // -- Plan-level correlation: each scheduler-side event carries the
    // plan_id and node_id RUN-02 names. node_id wiring is what lets the
    // dashboard scope events to the right node.
    let node_started: Vec<&Value> = emitted
        .iter()
        .filter(|(k, _)| k == EVENT_NODE_STARTED)
        .map(|(_, p)| p)
        .collect();
    assert_eq!(node_started.len(), 3);
    for payload in &node_started {
        assert_eq!(payload["plan_id"], plan_id.as_str());
        assert!(payload.get("node_id").is_some());
        assert!(payload.get("executor_kind").is_some());
    }

    // Every started node id is one of the three real ones.
    let started_ids: Vec<String> = node_started
        .iter()
        .map(|p| p["node_id"].as_str().unwrap().to_string())
        .collect();
    assert!(started_ids.contains(&script_id.to_string()));
    assert!(started_ids.contains(&agent_id.to_string()));
    assert!(started_ids.contains(&verify_id.to_string()));
}

#[tokio::test]
async fn verify_failure_still_emits_synodic_verdict() {
    // A verify node with a failing check + Escalate policy emits a
    // `synodic.verdict` (passed=false) AND a `node.failed`. The
    // contract is "verdict emits regardless of pass/fail" — the
    // dashboard renders the verdict row even on the failure path so a
    // human can see why the run halted.
    let e_in = EdgeId::generate();
    let e_out = EdgeId::generate();
    let node_id = NodeId::generate();
    let plan = ExecutionPlan {
        nodes: vec![Node {
            id: node_id,
            executor: Box::new(VerifyExecutor::with_checks(
                vec![Check::Lint {
                    name: "clippy".into(),
                    must_pass: false,
                }],
                FailPolicy::Escalate,
            )),
            inputs: vec![EdgeRef::new(e_in)],
            outputs: vec![EdgeRef::new(e_out)],
        }],
        edges: vec![
            Edge {
                id: e_in,
                artifact_id: ArtifactId::new("upstream"),
                requires_deterministic: false,
            },
            Edge {
                id: e_out,
                artifact_id: ArtifactId::new("downstream"),
                requires_deterministic: false,
            },
        ],
        spec_index: Default::default(),
    };

    // Per-node executor configuration is not yet routed through the
    // registry-backed dispatch (v1 holds one instance per kind; see
    // RUN-01's `crate::dispatch` note). For the failure-path test
    // the *registry* instance carries the failing check; the
    // substrate-side `Node::executor` value is a serialization-only
    // placeholder that hands off to the registered instance.
    let mut reg = ExecutorRegistry::new();
    reg.register(Arc::new(VerifyExecutor::with_checks(
        vec![Check::Lint {
            name: "clippy".into(),
            must_pass: false,
        }],
        FailPolicy::Escalate,
    )));
    let store = Arc::new(InMemoryPlanStore::new());
    let spine = Arc::new(CapturingSpine::default());
    let scheduler = Scheduler::new(
        Arc::new(reg),
        Arc::clone(&store) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    );

    let _ = scheduler.run(&PlanId::generate(), &plan).await;

    let emitted = spine.emitted.lock().unwrap().clone();
    let verdict = emitted
        .iter()
        .find(|(k, _)| k == se::KIND_SYNODIC_VERDICT)
        .expect("synodic.verdict must emit on the failure path too");
    assert_eq!(verdict.1["passed"], false);

    let failed = emitted
        .iter()
        .find(|(k, _)| k == EVENT_NODE_FAILED)
        .expect("Escalate policy must surface node.failed");
    assert!(failed.1["error"].as_str().unwrap().contains("escalating"));
}
