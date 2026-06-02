//! End-to-end scheduler agent-loop test (spec #537, closing #533's
//! headline acceptance bullet).
//!
//! Proves the full wire: a [`SpecPlan`] whose compiled DAG contains one
//! agent node runs to terminal through [`PlanRunner`] →
//! [`onsager_nodes::Scheduler`] → `dispatch` → the *production*
//! [`AgentExecutor`] registered by [`default_executor_registry`] →
//! [`onsager_scheduler::ClaudeCliRunner`] spawning a sandboxed fake
//! `claude` (no live API) → the §4c authoritative liveness gate reading
//! the session manifest back → a packaged `Artifact`
//! (`Uncertain { source: Agent }`) on the node's output edge.
//!
//! Four tests:
//!
//! 1. `agent_node_runs_to_terminal_with_uncertain_agent_artifact` — the
//!    §520 acceptance bullet, now on the scheduler path, against the
//!    real `default_executor_registry()`.
//! 2. `agent_node_fails_unknown_kind_when_executor_unregistered` — the
//!    mutation check: drop `AgentExecutor` from the registry and the
//!    same plan fails `UnknownKind`, proving the registration is
//!    load-bearing (not theater).
//! 3. `scheduler_path_injects_session_token_and_mcp_into_agent_spawn` —
//!    the §536 plan-scoped token + §531 MCP config + §4d Stop hook are
//!    wired on the scheduler path: with a plan token threaded through
//!    `PlanRunner` and the MCP / liveness-hook / workspace env present,
//!    the spawned agent carries `ONSAGER_SESSION_TOKEN`, `--mcp-config`,
//!    and `--settings` (Stop hook). The token's presence is exactly what
//!    lets the §4d hook authenticate instead of 401→fail-open.
//! 4. `scheduler_path_surfaces_repos_to_agent_spawn` — the #555
//!    multi-repo read parity: a run for a workspace with two bound repos
//!    threads `ONSAGER_REPOS` through `PlanRunner` to the spawned agent,
//!    which sees both authenticated clone URLs.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use onsager_agent_spawn::{ClonableRepo, LIVENESS_HOOK_URL_ENV, MCP_URL_ENV, repos_env_json};
use onsager_artifact::{Artifact, ArtifactId, ContentRef, NodeId, Provenance, SourceTag};
use onsager_nodes::{
    AgentExecutor, EmittedArtifact, ExecutorRegistry, InMemoryPlanStore, NoOpExecutor, PlanId,
    PlanStore, SessionManifest, SpineClient, SpineError,
};
use onsager_scheduler::{PlanRunner, service::default_executor_registry};
use onsager_substrate::WorkflowLookup;
use onsager_substrate::ids::{EdgeId, WorkflowId};
use onsager_substrate::spec_plan::{SpecId, SpecPlan, SpecRef};
use onsager_substrate::workflow::{Edge, EdgeRef, Node, OutputSpec, Workflow};

/// Serializes the env-mutating tests. `default_executor_registry()`
/// reads `ONSAGER_CLAUDE_BIN`, and `ClaudeCliRunner::run` reads the MCP /
/// liveness-hook / workspace env at spawn time; both touch the
/// process-global env. A `tokio::sync::Mutex` so the guard survives the
/// `.await` without tripping `clippy::await_holding_lock`.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const CLAUDE_BIN_ENV: &str = "ONSAGER_CLAUDE_BIN";
const WORKSPACE_ID_ENV: &str = "ONSAGER_WORKSPACE_ID";

/// Spine that records every emit and serves a single-emit manifest for
/// every session — the agent "delivered" via `emit_artifact`, so the
/// §4c gate packages that one output. Mirrors the crawlab fixture's
/// `ObservingSpine` shape.
#[derive(Debug, Default)]
struct ManifestSpine {
    emitted: Mutex<Vec<(String, serde_json::Value)>>,
}

#[async_trait]
impl SpineClient for ManifestSpine {
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
    async fn read_session_manifest(&self, session_id: &str) -> Result<SessionManifest, SpineError> {
        Ok(SessionManifest {
            emitted: vec![EmittedArtifact {
                artifact_id: format!("art_{session_id}"),
                content_ref: ContentRef {
                    uri: "inline:base64,YW5hbHlzaXM=".into(),
                    checksum: None,
                },
                kind: "document".into(),
                summary: "scheduler-path agent analysis".into(),
            }],
            declared_empty: false,
        })
    }
}

/// One-kind library: `agent-analysis` → a workflow with a single agent
/// node, no inputs, one (non-deterministic) output edge declaring
/// `Uncertain { Agent }` — exactly what the agent emits.
struct AgentLibrary {
    workflow: Workflow,
}

impl WorkflowLookup for AgentLibrary {
    fn get(&self, _: WorkflowId) -> Option<&Workflow> {
        None
    }
    fn by_kind(&self, kind: &str) -> Option<&Workflow> {
        (kind == "agent-analysis").then_some(&self.workflow)
    }
}

/// Build the single-agent-node workflow. The node's own
/// `AgentExecutor` carries the per-node model / prompt the scheduler
/// threads to the registered singleton via `ExecutorContext::agent_config`
/// (#534); its `runner` is irrelevant (dispatch runs the registry
/// instance), so it keeps the default `UnconfiguredRunner`.
fn agent_workflow() -> Workflow {
    let exit = EdgeId::generate();
    let agent = AgentExecutor::new(
        "claude-sonnet-4-6",
        "you are a code-review agent — analyse the input",
    );
    Workflow {
        nodes: vec![Node {
            id: NodeId::generate(),
            executor: Box::new(agent),
            inputs: vec![],
            outputs: vec![EdgeRef::new(exit)],
        }],
        edges: vec![Edge {
            id: exit,
            artifact_id: ArtifactId::new("analysis"),
            // Agent emits Uncertain — this edge cannot demand Deterministic.
            requires_deterministic: false,
        }],
        entry_specs: vec![],
        output_specs: vec![OutputSpec {
            edge_id: exit,
            provenance: Provenance::Uncertain {
                source: SourceTag::Agent,
            },
        }],
    }
}

fn one_agent_spec_plan() -> SpecPlan {
    SpecPlan {
        specs: vec![SpecRef {
            id: SpecId::new("analyse"),
            kind: "agent-analysis".into(),
            inputs: Default::default(),
        }],
        deps: vec![],
    }
}

/// Write an executable fake `claude` that records its argv + the env
/// vars the assertions care about to `capture_path`, then emits a canned
/// success `Result` on stdout. Returns the temp dir (kept alive) and the
/// script path.
fn fake_claude(capture_path: &std::path::Path) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-claude.sh");
    let mut f = std::fs::File::create(&script).unwrap();
    write!(
        f,
        "#!/usr/bin/env bash\n\
         {{\n\
           echo \"ARGS: $*\"\n\
           echo \"SESSION_TOKEN: ${{ONSAGER_SESSION_TOKEN:-}}\"\n\
           echo \"REPOS: ${{ONSAGER_REPOS:-}}\"\n\
         }} >> \"{capture}\"\n\
         echo '{{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"analysis done\"}}'\n",
        capture = capture_path.display(),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    (dir, script.to_string_lossy().into_owned())
}

/// Pull the first `node.completed.output_artifact_ids[0]` off a recorded
/// emit stream.
fn first_output_id(emitted: &[(String, serde_json::Value)]) -> Option<String> {
    emitted
        .iter()
        .find(|(k, _)| k == "node.completed")
        .and_then(|(_, p)| p.get("output_artifact_ids"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[tokio::test]
async fn agent_node_runs_to_terminal_with_uncertain_agent_artifact() {
    let _guard = ENV_LOCK.lock().await;
    let cap = tempfile::tempdir().unwrap();
    let cap_file = cap.path().join("argv.txt");
    let (_dir, script) = fake_claude(&cap_file);

    // Point the *production* registry's ClaudeCliRunner at the fixture.
    // SAFETY: serialized by ENV_LOCK; removed before the guard drops.
    unsafe {
        std::env::set_var(CLAUDE_BIN_ENV, &script);
    }

    let spine = Arc::new(ManifestSpine::default());
    let store = Arc::new(InMemoryPlanStore::new());
    let registry = Arc::new(default_executor_registry());
    let runner = PlanRunner::new(
        Arc::clone(&registry),
        Arc::clone(&store) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    );

    let plan_id = PlanId::generate();
    let result = runner
        .run(
            &plan_id,
            &one_agent_spec_plan(),
            &AgentLibrary {
                workflow: agent_workflow(),
            },
        )
        .await;

    unsafe {
        std::env::remove_var(CLAUDE_BIN_ENV);
    }
    result.expect("agent-node plan runs to terminal");

    // node.completed.output_artifact_ids is non-empty.
    let emitted = spine.emitted.lock().unwrap().clone();
    let output_id =
        first_output_id(&emitted).expect("a node.completed with a non-empty output_artifact_ids");

    // The packaged artifact is Uncertain { source: Agent } — stamped by
    // the §4c gate, never trusted from the manifest.
    let art = store
        .get_artifact(&plan_id, &ArtifactId::new(&output_id))
        .await
        .expect("store read ok")
        .expect("the agent output artifact is persisted on its edge");
    assert_eq!(
        art.provenance,
        Provenance::Uncertain {
            source: SourceTag::Agent,
        },
        "scheduler-dispatched agent output must be Uncertain {{ Agent }}",
    );
}

#[tokio::test]
async fn agent_node_fails_unknown_kind_when_executor_unregistered() {
    // Mutation check (not theater): the same plan against a registry
    // with NO AgentExecutor (the old NoOp-only default) fails
    // `UnknownKind` — proving the #537 registration is load-bearing. No
    // claude is spawned (dispatch fails before reaching the runner), so
    // this needs no env.
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(NoOpExecutor));
    let runner = PlanRunner::new(
        Arc::new(registry),
        Arc::new(InMemoryPlanStore::new()) as Arc<dyn PlanStore>,
        Arc::new(ManifestSpine::default()) as Arc<dyn SpineClient>,
    );

    let plan_id = PlanId::generate();
    let err = runner
        .run(
            &plan_id,
            &one_agent_spec_plan(),
            &AgentLibrary {
                workflow: agent_workflow(),
            },
        )
        .await
        .expect_err("an agent node must fail when AgentExecutor is unregistered");
    let msg = err.to_string();
    assert!(
        msg.contains("agent"),
        "the failure should name the unknown `agent` kind, got: {msg}",
    );
}

#[tokio::test]
async fn scheduler_path_injects_session_token_and_mcp_into_agent_spawn() {
    // §536 token + §531 MCP + §4d Stop hook, on the scheduler path. With
    // a plan-scoped token threaded through PlanRunner and the MCP /
    // liveness-hook / workspace env present, the spawned agent must carry
    // ONSAGER_SESSION_TOKEN, --mcp-config, and --settings (the Stop
    // hook). The token's presence is precisely what lets the §4d hook
    // authenticate instead of 401→fail-open.
    let _guard = ENV_LOCK.lock().await;
    let cap = tempfile::tempdir().unwrap();
    let cap_file = cap.path().join("argv.txt");
    let (_dir, script) = fake_claude(&cap_file);

    // SAFETY: serialized by ENV_LOCK; all removed before the guard drops.
    unsafe {
        std::env::set_var(CLAUDE_BIN_ENV, &script);
        std::env::set_var(MCP_URL_ENV, "https://app.example.com/mcp/messages");
        std::env::set_var(LIVENESS_HOOK_URL_ENV, "https://app.example.com/agent/stop");
        std::env::set_var(WORKSPACE_ID_ENV, "ws-7");
    }

    let spine = Arc::new(ManifestSpine::default());
    let runner = PlanRunner::new(
        Arc::new(default_executor_registry()),
        Arc::new(InMemoryPlanStore::new()) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    )
    .with_session_token(Some("ons_pat_plan_scoped".to_string()));

    let plan_id = PlanId::generate();
    let result = runner
        .run(
            &plan_id,
            &one_agent_spec_plan(),
            &AgentLibrary {
                workflow: agent_workflow(),
            },
        )
        .await;

    unsafe {
        std::env::remove_var(CLAUDE_BIN_ENV);
        std::env::remove_var(MCP_URL_ENV);
        std::env::remove_var(LIVENESS_HOOK_URL_ENV);
        std::env::remove_var(WORKSPACE_ID_ENV);
    }
    result.expect("agent-node plan runs to terminal");

    let recorded = std::fs::read_to_string(&cap_file).unwrap();
    assert!(
        recorded.contains("SESSION_TOKEN: ons_pat_plan_scoped"),
        "the plan-scoped token must reach the agent env as ONSAGER_SESSION_TOKEN, got: {recorded}",
    );
    assert!(
        recorded.contains("--mcp-config"),
        "the MCP server must be registered when URL + token are present, got: {recorded}",
    );
    assert!(
        recorded.contains("--settings"),
        "the §4d Stop hook (--settings) must be wired on the scheduler path, got: {recorded}",
    );
}

#[tokio::test]
async fn scheduler_path_surfaces_repos_to_agent_spawn() {
    // §555: a scheduler-launched run for a workspace with ≥2 bound repos
    // must surface BOTH in ONSAGER_REPOS with authenticated clone URLs —
    // the multi-repo read parity the chat path already had. The set is
    // built once per plan and threaded PlanRunner → Scheduler →
    // ExecutorContext → AgentExecutor → AgentRequest → ClaudeCliRunner →
    // the spawned agent's env. Mutation guard: drop a repo from
    // `repos_env` (or the `with_repos_env` thread) and an assertion below
    // fails. (The decrypt-off-the-event half is unit-tested in
    // `onsager_scheduler::repo_env`.)
    let _guard = ENV_LOCK.lock().await;
    let cap = tempfile::tempdir().unwrap();
    let cap_file = cap.path().join("argv.txt");
    let (_dir, script) = fake_claude(&cap_file);

    // SAFETY: serialized by ENV_LOCK; removed before the guard drops.
    unsafe {
        std::env::set_var(CLAUDE_BIN_ENV, &script);
    }

    // The agent-facing shape the scheduler builds from the decrypted repo
    // set — two bound repos, each with a read token.
    let repos_env = repos_env_json(&[
        ClonableRepo {
            owner: "acme".into(),
            name: "widgets".into(),
            default_branch: "main".into(),
            token: Some("tok_a".into()),
        },
        ClonableRepo {
            owner: "other".into(),
            name: "thing".into(),
            default_branch: "main".into(),
            token: Some("tok_b".into()),
        },
    ])
    .expect("two repos build a value");

    let spine = Arc::new(ManifestSpine::default());
    let runner = PlanRunner::new(
        Arc::new(default_executor_registry()),
        Arc::new(InMemoryPlanStore::new()) as Arc<dyn PlanStore>,
        Arc::clone(&spine) as Arc<dyn SpineClient>,
    )
    .with_repos_env(Some(repos_env));

    let plan_id = PlanId::generate();
    let result = runner
        .run(
            &plan_id,
            &one_agent_spec_plan(),
            &AgentLibrary {
                workflow: agent_workflow(),
            },
        )
        .await;

    unsafe {
        std::env::remove_var(CLAUDE_BIN_ENV);
    }
    result.expect("agent-node plan runs to terminal");

    let recorded = std::fs::read_to_string(&cap_file).unwrap();
    // Both repos surface, each with an authenticated (x-access-token)
    // clone URL — "surfaces both with read-auth", on the scheduler path.
    assert!(
        recorded.contains("https://x-access-token:tok_a@github.com/acme/widgets.git"),
        "the first bound repo must reach the agent env via ONSAGER_REPOS, got: {recorded}",
    );
    assert!(
        recorded.contains("https://x-access-token:tok_b@github.com/other/thing.git"),
        "the second bound repo must reach the agent env via ONSAGER_REPOS, got: {recorded}",
    );
}
