//! [`AgentExecutor`] — the LLM-agent executor.
//!
//! Per issue #355 (EXE-03), the agent executor "colors" its outputs:
//! regardless of input provenance, the artifact it produces is
//! [`Provenance::Uncertain { source: Agent }`]. Invariant 2 of ADR 0018
//! then guarantees any downstream non-Verify node sees the Uncertain
//! propagate — Uncertain is contagious.
//!
//! This module ports the orchestration shape from
//! `stiglab/src/core/session.rs` (plus the actual subprocess plumbing in
//! `stiglab/src/agent/session/process.rs`) into an `Executor`. The note
//! on the issue is explicit: the stiglab session code stays in place
//! until MIG-01 (#363) — this is a port/addition.
//!
//! The actual Claude session call is abstracted behind [`AgentRunner`]
//! so tests can drive `execute` with a canned response and a real
//! production binary wires in the live runner. Runner choice is
//! intentionally not part of the serializable shape — the substrate
//! side of `AgentExecutor` carries configuration (model, prompt,
//! tools, credential reference) while runtime wiring happens at
//! `ExecutorRegistry::register` time.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use onsager_artifact::{Artifact, ArtifactVersion, Kind, Provenance, SourceTag};
use onsager_substrate::events as se;
use onsager_substrate::executor::Executor as SubstrateExecutor;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor;
use crate::spine::{EmittedArtifact, SessionManifest};

/// LLM-agent executor.
///
/// Configuration mirrors the fields stiglab's `Task` carries today
/// (`stiglab/src/core/task.rs`): `model`, `system_prompt`, allowed
/// `tools`, and a credential bundle reference. The agent reads its
/// upstream artifacts off [`ExecutorContext::inputs`], asks the
/// configured [`AgentRunner`] to run a session, and packages the
/// response into a single output [`Artifact`] tagged
/// [`Provenance::Uncertain { source: Agent }`].
///
/// The `runner` field is `#[serde(skip)]` — it's a runtime wiring
/// concern, not part of the workflow template's serialized form. A
/// deserialized `AgentExecutor` carries the [`UnconfiguredRunner`]
/// default and will error if `execute` is called before
/// `with_runner(..)` rewires it. The substrate-side validation path
/// (declared_provenance / kind lookup) does not touch `runner`, so
/// pre-flight invariant checks work on freshly-deserialized templates
/// unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecutor {
    pub model: String,
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
    #[serde(skip, default = "default_runner")]
    runner: Arc<dyn AgentRunner>,
}

fn default_runner() -> Arc<dyn AgentRunner> {
    Arc::new(UnconfiguredRunner)
}

impl AgentExecutor {
    /// Build an `AgentExecutor` with the bare model + system prompt
    /// configuration. Tools, credentials, and runner default to empty
    /// / unconfigured; layer them in with the `with_*` builders.
    pub fn new(model: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: system_prompt.into(),
            tools: Vec::new(),
            credential_ref: None,
            runner: default_runner(),
        }
    }

    /// Replace the runtime runner. Required before `execute` does
    /// anything useful — the default is [`UnconfiguredRunner`].
    pub fn with_runner(mut self, runner: Arc<dyn AgentRunner>) -> Self {
        self.runner = runner;
        self
    }

    /// Set the allowed tool list this agent session may dispatch.
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools = tools;
        self
    }

    /// Set the credential bundle reference (e.g., a workspace-scoped
    /// credentials row id) the runner should hydrate before calling
    /// the LLM.
    pub fn with_credential_ref(mut self, credential_ref: impl Into<String>) -> Self {
        self.credential_ref = Some(credential_ref.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Runner abstraction
// ---------------------------------------------------------------------------

/// Request handed to an [`AgentRunner`] when [`AgentExecutor::execute`]
/// dispatches a session.
#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub credential_ref: Option<String>,
    /// Assembled user-side prompt body — one section per input
    /// artifact. The v1 assembler is intentionally crude; the full
    /// templating story lands with the scheduler (RUN-01, #359).
    pub user_prompt: String,
    /// Identifier for this agent session. The executor mints it and
    /// hands it to the runner so the agent's execution environment
    /// attributes its `emit_artifact` / `declare_no_output` MCP calls
    /// to the same `session:<id>` manifest stream the executor reads
    /// back authoritatively (spec #520 §4b/§4c).
    pub session_id: String,
}

/// Response returned by an [`AgentRunner`].
///
/// The agent session's final assistant text is carried in `output`.
/// Per spec #520 §4b the response is *widened* to also surface the
/// outputs the session emitted via `emit_artifact` — an **optimization
/// channel only**. The authoritative source of "what came out" is the
/// `events_ext` manifest, which [`AgentExecutor::execute`] reads back
/// directly (spec #520 §4c). A runner that can't cheaply surface the
/// emitted refs (e.g. the stiglab NDJSON stdout stream, where emits
/// flow out-of-band through MCP) leaves `emitted` empty and the gate
/// still works off the manifest.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub output: String,
    /// Outputs the runner observed the session emit. Optimization only
    /// — never the authoritative source (see struct docs); the
    /// executor cross-checks it against the manifest and warns on
    /// divergence.
    pub emitted: Vec<EmittedArtifact>,
}

/// Error returned by an [`AgentRunner::run`] call.
#[derive(Debug, Error)]
#[error("agent runner error: {0}")]
pub struct AgentRunError(String);

impl AgentRunError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Port over the actual LLM session backend.
///
/// Real production runners spawn the `claude` CLI as a subprocess (the
/// shape stiglab uses today — see `stiglab/src/agent/session/process.rs`)
/// or call the Anthropic API directly. The trait keeps the executor
/// testable without either dependency: tests use [`StubAgentRunner`]
/// to assert provenance behavior with no network or process spawn.
///
/// Object-safe by design: held inside `AgentExecutor` as
/// `Arc<dyn AgentRunner>`.
#[async_trait]
pub trait AgentRunner: Send + Sync + std::fmt::Debug {
    async fn run(&self, request: AgentRequest) -> Result<AgentResponse, AgentRunError>;
}

/// Default runner placeholder.
///
/// `AgentExecutor` carries one of these whenever it was deserialized
/// rather than constructed in code (because the runner is `#[serde(skip)]`).
/// Calling `run` always errors with a message pointing the operator at
/// `with_runner`. Substrate-side validation never touches the runner,
/// so this default is invisible to the kernel invariant checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnconfiguredRunner;

#[async_trait]
impl AgentRunner for UnconfiguredRunner {
    async fn run(&self, _request: AgentRequest) -> Result<AgentResponse, AgentRunError> {
        Err(AgentRunError::new(
            "AgentExecutor has no configured runner — call `with_runner(..)` before registering",
        ))
    }
}

/// In-memory runner for tests and as a stub in early-bringup wiring.
/// Returns the configured `output` for every call.
#[derive(Debug, Clone)]
pub struct StubAgentRunner {
    pub output: String,
}

impl StubAgentRunner {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

#[async_trait]
impl AgentRunner for StubAgentRunner {
    async fn run(&self, _request: AgentRequest) -> Result<AgentResponse, AgentRunError> {
        Ok(AgentResponse {
            output: self.output.clone(),
            // The stub reports "session over" only — the manifest (set
            // up by the test's `MockSpine`) is the authoritative source
            // the gate reads (spec #520 §4c).
            emitted: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Substrate side (sync, serializable) — what nodes carry on the wire
// ---------------------------------------------------------------------------

#[typetag::serde(name = "agent")]
impl SubstrateExecutor for AgentExecutor {
    fn executor_kind(&self) -> &'static str {
        "agent"
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        // The agent "colors" its output regardless of input
        // provenance — issue #355, ADR 0010 invariant 2.
        Provenance::Uncertain {
            source: SourceTag::Agent,
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime side (async) — what the dispatcher actually invokes
// ---------------------------------------------------------------------------

#[async_trait]
impl Executor for AgentExecutor {
    fn executor_kind(&self) -> &'static str {
        "agent"
    }

    fn declared_provenance(&self, _inputs: &[Provenance]) -> Provenance {
        Provenance::Uncertain {
            source: SourceTag::Agent,
        }
    }

    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        // RUN-02 (#360) lifecycle events. `plan_id` comes off
        // `ExecutorContext`, populated by the scheduler so the
        // spine envelope's `stream_id` (`plan:<plan>:<node>`)
        // correlates verdicts back to a specific run.
        let plan_id = ctx.plan_id.as_str().to_string();
        let session_id = Uuid::new_v4().to_string();

        let request = AgentRequest {
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            tools: self.tools.clone(),
            credential_ref: self.credential_ref.clone(),
            user_prompt: render_user_prompt(&ctx),
            session_id: session_id.clone(),
        };

        emit_event(
            &ctx,
            se::KIND_AGENT_SESSION_STARTED,
            &se::AgentSessionStarted {
                plan_id: plan_id.clone(),
                node_id: ctx.node_id,
                session_id: session_id.clone(),
                model: self.model.clone(),
            },
        )
        .await;

        let response = match self.runner.run(request).await {
            Ok(r) => r,
            Err(e) => {
                emit_event(
                    &ctx,
                    se::KIND_AGENT_SESSION_FAILED,
                    &se::AgentSessionFailed {
                        plan_id: plan_id.clone(),
                        node_id: ctx.node_id,
                        session_id: session_id.clone(),
                        error: e.to_string(),
                    },
                )
                .await;
                return Err(ExecutorError::failed(e.to_string()));
            }
        };

        emit_event(
            &ctx,
            se::KIND_AGENT_SESSION_COMPLETED,
            &se::AgentSessionCompleted {
                plan_id: plan_id.clone(),
                node_id: ctx.node_id,
                session_id: session_id.clone(),
                // Token usage is not surfaced through `AgentResponse`
                // yet; the live runner will fill it in via a runner-
                // side hook in a follow-up. Leaving `None` keeps the
                // budget consumer honest — "not reported" ≠ "zero".
                token_usage: None,
            },
        )
        .await;

        // ---------------------------------------------------------------
        // Authoritative liveness gate (spec #520 §4c).
        //
        // The manifest — not the runner's response — is the source of
        // truth for "what came out". Read it back over the spine port;
        // the deployed adapter queries the `events_ext` `session:<id>`
        // stream the agent's MCP `emit_artifact` / `declare_no_output`
        // calls wrote to. This judgment does not depend on the in-
        // session Stop hook (§4d) having fired.
        // ---------------------------------------------------------------
        let manifest = ctx
            .spine
            .read_session_manifest(&session_id)
            .await
            .map_err(|e| {
                ExecutorError::failed(format!(
                    "agent liveness gate: failed to read session manifest for {session_id}: {e}"
                ))
            })?;

        // The widened response (spec #520 §4b) is an optimization
        // channel only; cross-check it against the authoritative
        // manifest and warn on divergence rather than trusting it.
        cross_check_response(&ctx, &response, &manifest);

        if manifest.emitted.is_empty() {
            if manifest.declared_empty {
                // Declared empty on purpose — finish cleanly with no
                // artifacts, flagging the intentional empty delivery so
                // the dashboard / scheduler can distinguish it from a
                // silent nothing (§4e `node.completed.declared_empty`).
                return Ok(ExecutorOutputs::declared_empty());
            }
            // Neither delivered nor declared empty — escalate. Reuse the
            // existing human-escalation channel (do not invent a new
            // one) and park the run via the Escalate-flavored
            // `ExecutorError::Failed`, consistent with
            // `VerifyExecutor`'s `FailPolicy::Escalate`.
            emit_event(
                &ctx,
                se::KIND_NODE_AWAITING_HUMAN,
                &se::NodeAwaitingHuman {
                    plan_id,
                    node_id: ctx.node_id,
                    prompt: "Agent session ended with no output and no explicit no-output \
                             declaration — forgotten, or genuinely nothing to deliver? Confirm."
                        .to_string(),
                },
            )
            .await;
            return Err(ExecutorError::failed(format!(
                "agent liveness gate: escalating — session {session_id} ended with no emitted \
                 output and no declare_no_output; parked for human decision"
            )));
        }

        // Non-empty: package one `Artifact` per emitted record. Each is
        // stamped `Provenance::Uncertain { source: Agent }` HERE — never
        // read from the manifest. Only a Verify node may upgrade
        // provenance (ADR 0010 / ADR 0018 invariant 2); liveness is not
        // a Verify node.
        let artifacts: Vec<_> = manifest
            .emitted
            .iter()
            .map(|rec| {
                let (id, art) = build_emitted_artifact(self, ctx.node_id, &session_id, rec);
                (id, art)
            })
            .collect();
        Ok(ExecutorOutputs {
            artifacts,
            declared_empty: None,
        })
    }
}

/// Best-effort spine emit for an executor lifecycle event. A failed
/// emit logs a warning and is not propagated — a missed lifecycle
/// event must not stall the actual execution (the artifact still
/// materializes; the dashboard timeline just loses one row).
/// `serde_json::to_value` on a substrate event struct cannot fail
/// (only string keys, no NaN floats).
async fn emit_event<T: serde::Serialize>(ctx: &ExecutorContext, kind: &str, payload: &T) {
    let payload = serde_json::to_value(payload).expect("substrate event payload must serialize");
    if let Err(e) = ctx.spine.emit(kind, payload).await {
        tracing::warn!(
            plan = %ctx.plan_id,
            node = %ctx.node_id,
            kind,
            "agent executor spine emit failed: {e}",
        );
    }
}

/// Render upstream artifacts into a single user-side prompt body. The
/// v1 form is one section per input artifact id; the scheduler
/// (RUN-01) replaces this with the full templating story.
fn render_user_prompt(ctx: &ExecutorContext) -> String {
    let mut out = String::new();
    for (id, _art) in &ctx.inputs {
        out.push_str("# input ");
        out.push_str(id.as_str());
        out.push('\n');
    }
    out
}

/// Build one DAG `Artifact` from a manifest-emitted record (spec #520
/// §4c). The agent-supplied `content_ref` lands on an
/// [`ArtifactVersion`]; provenance is stamped
/// `Provenance::Uncertain { source: Agent }` here — never trusted from
/// the manifest.
fn build_emitted_artifact(
    exec: &AgentExecutor,
    node_id: onsager_artifact::NodeId,
    session_id: &str,
    rec: &EmittedArtifact,
) -> (onsager_artifact::ArtifactId, Artifact) {
    let name = if rec.summary.is_empty() {
        "agent-output".to_string()
    } else {
        rec.summary.clone()
    };
    let mut artifact = Artifact::new(
        kind_from_str(&rec.kind),
        name,
        "agent",
        node_id.to_string(),
        Vec::new(),
    );
    // Preserve the identity minted at emit time (spec #520 §4a) rather
    // than the throwaway id `Artifact::new` generated, so the packaged
    // artifact matches the manifest record downstream can reference.
    // (The scheduler still re-keys this to the edge's namespaced
    // ArtifactId on persist — single-writer-per-edge — but the
    // executor's own output stays faithful to the manifest, which is
    // what direct (non-scheduler) callers and the unit tests observe.)
    artifact.artifact_id = onsager_artifact::ArtifactId::new(rec.artifact_id.clone());
    // Stamped HERE, not read from the manifest (spec #520 §4c, ADR 0010
    // / ADR 0018 invariant 2). If this were ever sourced from the
    // agent-controlled record, the agent could self-certify
    // `Deterministic` — the mutation test guards against exactly that.
    artifact.provenance = Executor::declared_provenance(exec, &[]);
    artifact.produced_by_node = Some(node_id);
    // Carry the agent-supplied content pointer on a first version.
    artifact.versions.push(ArtifactVersion {
        version: 1,
        created_at: Utc::now(),
        created_by_session: session_id.to_string(),
        content_ref: rec.content_ref.clone(),
        change_summary: rec.summary.clone(),
        parent_version: None,
    });
    artifact.current_version = 1;
    let id = artifact.artifact_id.clone();
    (id, artifact)
}

/// Map a manifest `kind` tag onto the [`Kind`] enum, mirroring
/// [`Kind`]'s `Display`. Unknown tags become [`Kind::Custom`] — the
/// kind set is typed but open.
fn kind_from_str(kind: &str) -> Kind {
    match kind {
        "code" => Kind::Code,
        "document" => Kind::Document,
        "pull_request" => Kind::PullRequest,
        "github_issue" => Kind::GithubIssue,
        other => Kind::Custom(other.to_string()),
    }
}

/// Cross-check the runner's optimization-channel `emitted` refs (spec
/// #520 §4b) against the authoritative manifest. The manifest always
/// wins; a divergence means the runner's fast-path view drifted from
/// the recorded truth — log it so the drift is visible without
/// stalling the run. A runner that reports nothing (the common case —
/// emits flow out-of-band through MCP) is silently fine.
fn cross_check_response(
    ctx: &ExecutorContext,
    response: &AgentResponse,
    manifest: &SessionManifest,
) {
    if response.emitted.is_empty() {
        return;
    }
    let mut from_response: Vec<&str> = response
        .emitted
        .iter()
        .map(|e| e.content_ref.uri.as_str())
        .collect();
    let mut from_manifest: Vec<&str> = manifest
        .emitted
        .iter()
        .map(|e| e.content_ref.uri.as_str())
        .collect();
    from_response.sort_unstable();
    from_manifest.sort_unstable();
    if from_response != from_manifest {
        tracing::warn!(
            plan = %ctx.plan_id,
            node = %ctx.node_id,
            response_emits = from_response.len(),
            manifest_emits = from_manifest.len(),
            "agent runner response.emitted diverged from the authoritative manifest; \
             using the manifest",
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ExecutorRegistry;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::{ArtifactId, ContentRef, NodeId};

    fn agent_with_stub(output: &str) -> AgentExecutor {
        AgentExecutor::new("claude-sonnet-4-6", "you are helpful")
            .with_runner(Arc::new(StubAgentRunner::new(output)))
    }

    /// A manifest carrying a single emitted output — the "agent
    /// delivered" case for the liveness gate.
    fn manifest_one_emit() -> SessionManifest {
        SessionManifest {
            emitted: vec![EmittedArtifact {
                artifact_id: "art_emit_1".into(),
                content_ref: ContentRef {
                    uri: "inline:base64,aGVsbG8=".into(),
                    checksum: Some("abc123".into()),
                },
                kind: "document".into(),
                summary: "the agent said hello".into(),
            }],
            declared_empty: false,
        }
    }

    /// A context whose spine serves `manifest` from
    /// `read_session_manifest`. Returns the concrete [`MockSpine`] too so
    /// tests can read back the lifecycle events the executor emitted.
    fn ctx_with_manifest(
        node_id: NodeId,
        manifest: SessionManifest,
    ) -> (ExecutorContext, Arc<MockSpine>) {
        let mock = Arc::new(MockSpine::with_manifest(manifest));
        let ctx = ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id,
            inputs: Vec::new(),
            spine: Arc::clone(&mock) as Arc<dyn crate::SpineClient>,
            subworkflow_ref: None,
        };
        (ctx, mock)
    }

    fn empty_ctx() -> ExecutorContext {
        // Default MockSpine serves an empty (no-emit, not-declared)
        // manifest — the liveness-gate trip condition. Tests that
        // expect `execute` to reach the gate use the manifest-aware
        // constructors above instead.
        ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id: NodeId::generate(),
            inputs: Vec::new(),
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        }
    }

    #[test]
    fn executor_kind_is_agent() {
        let exec = AgentExecutor::new("model", "prompt");
        assert_eq!(Executor::executor_kind(&exec), "agent");
        assert_eq!(SubstrateExecutor::executor_kind(&exec), "agent");
    }

    #[test]
    fn declared_provenance_is_always_uncertain_agent() {
        let exec = AgentExecutor::new("model", "prompt");
        // No inputs at all.
        assert_eq!(
            Executor::declared_provenance(&exec, &[]),
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
        // Deterministic Script input — agent still colors.
        assert_eq!(
            Executor::declared_provenance(
                &exec,
                &[Provenance::Deterministic {
                    source: SourceTag::Script
                }]
            ),
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
        // Uncertain Human input — agent claims its own source.
        assert_eq!(
            Executor::declared_provenance(
                &exec,
                &[Provenance::Uncertain {
                    source: SourceTag::Human
                }]
            ),
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
        // The substrate side must agree — both halves share the
        // same provenance contract.
        assert_eq!(
            SubstrateExecutor::declared_provenance(&exec, &[]),
            Executor::declared_provenance(&exec, &[]),
        );
    }

    // -----------------------------------------------------------------
    // Authoritative liveness gate (spec #520 §4c).
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn execute_packages_emitted_manifest_record_as_uncertain_agent_artifact() {
        // Stub runner reports "session over"; the manifest carries one
        // emitted output. The gate packages it onto the DAG.
        let exec = agent_with_stub("final assistant text");
        let node_id = NodeId::generate();
        let (ctx, _spine) = ctx_with_manifest(node_id, manifest_one_emit());

        let outputs = exec.execute(ctx).await.unwrap();
        // node.completed.output_artifact_ids will be non-empty.
        assert_eq!(outputs.artifacts.len(), 1);
        assert_eq!(outputs.declared_empty, None);
        let (_id, art) = &outputs.artifacts[0];
        // Mutation guard: provenance is STAMPED here as
        // Uncertain { Agent }, never read from the manifest. Forcing
        // this to `Deterministic` in `build_emitted_artifact` fails
        // this assertion (spec #520 §4c).
        assert_eq!(
            art.provenance,
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
        assert_eq!(art.produced_by_node, Some(node_id));
        // The minted manifest id is preserved (not a throwaway ULID).
        assert_eq!(art.artifact_id.as_str(), "art_emit_1");
        // The agent-supplied content_ref rides on a first version.
        assert_eq!(art.current_version, 1);
        assert_eq!(art.versions.len(), 1);
        assert_eq!(art.versions[0].content_ref.uri, "inline:base64,aGVsbG8=");
        assert_eq!(art.kind, Kind::Document);
    }

    #[tokio::test]
    async fn execute_escalates_when_empty_and_not_declared() {
        // No emits, no declaration → emit node.awaiting_human + return
        // the Escalate path.
        let exec = agent_with_stub("i did some thinking but never emitted");
        let node_id = NodeId::generate();
        let (ctx, spine) = ctx_with_manifest(node_id, SessionManifest::default());

        let err = exec.execute(ctx).await.unwrap_err();
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("escalating"), "{msg}");
                assert!(msg.contains("no emitted output"), "{msg}");
            }
            other => panic!("expected ExecutorError::Failed, got {other:?}"),
        }

        // node.awaiting_human was emitted on the existing escalation
        // channel.
        let emitted = spine.emitted.lock().unwrap();
        assert!(
            emitted
                .iter()
                .any(|(k, _)| k == se::KIND_NODE_AWAITING_HUMAN),
            "expected node.awaiting_human emit, got: {emitted:?}"
        );
    }

    #[tokio::test]
    async fn execute_finishes_clean_when_declared_empty() {
        // No emits but an explicit declaration → clean finish, no
        // artifacts, no escalation, declared_empty signal set.
        let exec = agent_with_stub("nothing to deliver this time");
        let node_id = NodeId::generate();
        let manifest = SessionManifest {
            emitted: Vec::new(),
            declared_empty: true,
        };
        let (ctx, spine) = ctx_with_manifest(node_id, manifest);

        let outputs = exec.execute(ctx).await.unwrap();
        assert!(outputs.artifacts.is_empty());
        // Surfaces as node.completed.declared_empty = Some(true) (§4e).
        assert_eq!(outputs.declared_empty, Some(true));
        // No escalation.
        let emitted = spine.emitted.lock().unwrap();
        assert!(
            !emitted
                .iter()
                .any(|(k, _)| k == se::KIND_NODE_AWAITING_HUMAN),
            "declared-empty must not escalate, got: {emitted:?}"
        );
    }

    #[tokio::test]
    async fn execute_renders_inputs_into_user_prompt() {
        // A runner that captures the request so we can read the
        // assembled user prompt.
        #[derive(Debug, Default)]
        struct CapturingRunner {
            request: std::sync::Mutex<Option<AgentRequest>>,
        }
        #[async_trait]
        impl AgentRunner for CapturingRunner {
            async fn run(&self, request: AgentRequest) -> Result<AgentResponse, AgentRunError> {
                *self.request.lock().unwrap() = Some(request);
                Ok(AgentResponse {
                    output: "ok".into(),
                    emitted: Vec::new(),
                })
            }
        }

        let captured: Arc<CapturingRunner> = Arc::new(CapturingRunner::default());
        let exec = AgentExecutor::new("claude-sonnet-4-6", "you are helpful")
            .with_tools(vec!["edit".into(), "read".into()])
            .with_credential_ref("creds_42")
            .with_runner(captured.clone());

        let input_id = ArtifactId::new("art_input_1");
        let input_art = Artifact::new(Kind::Document, "upstream", "owner", "test", Vec::new());
        // Declared-empty manifest so the gate lets `execute` return Ok;
        // this test only asserts the assembled request.
        let mock = Arc::new(MockSpine::with_manifest(SessionManifest {
            emitted: Vec::new(),
            declared_empty: true,
        }));
        let ctx = ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id: NodeId::generate(),
            inputs: vec![(input_id.clone(), input_art)],
            spine: mock as Arc<dyn crate::SpineClient>,
            subworkflow_ref: None,
        };

        exec.execute(ctx).await.unwrap();

        let request = captured.request.lock().unwrap().clone().unwrap();
        assert_eq!(request.model, "claude-sonnet-4-6");
        assert_eq!(request.system_prompt, "you are helpful");
        assert_eq!(request.tools, vec!["edit".to_string(), "read".to_string()]);
        assert_eq!(request.credential_ref.as_deref(), Some("creds_42"));
        assert!(
            request.user_prompt.contains(input_id.as_str()),
            "user prompt should reference the upstream artifact id, got: {}",
            request.user_prompt
        );
        // The executor minted a session id and threaded it to the runner.
        assert!(
            !request.session_id.is_empty(),
            "session_id must be threaded to the runner (spec #520 §4b)"
        );
    }

    #[tokio::test]
    async fn execute_propagates_runner_error_as_failed() {
        #[derive(Debug)]
        struct ExplodingRunner;
        #[async_trait]
        impl AgentRunner for ExplodingRunner {
            async fn run(&self, _r: AgentRequest) -> Result<AgentResponse, AgentRunError> {
                Err(AgentRunError::new("api went down"))
            }
        }

        let exec = AgentExecutor::new("m", "p").with_runner(Arc::new(ExplodingRunner));
        let err = exec.execute(empty_ctx()).await.unwrap_err();
        match err {
            ExecutorError::Failed(msg) => assert!(
                msg.contains("api went down"),
                "expected runner message to surface, got: {msg}"
            ),
            other => panic!("expected ExecutorError::Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unconfigured_runner_errors_when_executed() {
        // No `with_runner` — the default carries `UnconfiguredRunner`.
        let exec = AgentExecutor::new("m", "p");
        let err = exec.execute(empty_ctx()).await.unwrap_err();
        match err {
            ExecutorError::Failed(msg) => {
                assert!(msg.contains("no configured runner"), "msg = {msg}")
            }
            other => panic!("expected ExecutorError::Failed, got {other:?}"),
        }
    }

    /// Script → Agent → Script chain verification from issue #355.
    ///
    /// Uses [`kernel_emit`] — a mirror of
    /// `onsager_substrate::validate::propagate_max_uncertainty`, which
    /// is private to the substrate validator. The mirror is the
    /// canonical kernel rule for non-Verify executors (ADR 0018
    /// invariant 2); duplicating it here is intentional so the chain
    /// claim is testable without standing up a full workflow.
    #[test]
    fn script_agent_script_chain_uncertain_is_contagious() {
        // Stage 1: a Script — declares Deterministic { Script }.
        let script1_declared = Provenance::Deterministic {
            source: SourceTag::Script,
        };
        let script1_emitted = kernel_emit(script1_declared, &[]);
        assert!(
            !script1_emitted.is_uncertain(),
            "Script with no upstream stays deterministic"
        );

        // Stage 2: Agent ingests Script output.
        let agent = AgentExecutor::new("claude-sonnet-4-6", "");
        let agent_declared = Executor::declared_provenance(&agent, &[script1_emitted]);
        let agent_emitted = kernel_emit(agent_declared, &[script1_emitted]);
        assert_eq!(
            agent_emitted,
            Provenance::Uncertain {
                source: SourceTag::Agent,
            },
            "Agent colors its output regardless of deterministic input"
        );

        // Stage 3: a downstream Script ingests the Agent output.
        // Its declared provenance is Deterministic, but invariant 2
        // upgrades the emitted to Uncertain because an Uncertain
        // input is present. The Agent source bubbles through.
        let script2_declared = Provenance::Deterministic {
            source: SourceTag::Script,
        };
        let script2_emitted = kernel_emit(script2_declared, &[agent_emitted]);
        assert!(
            script2_emitted.is_uncertain(),
            "Uncertain is contagious — downstream non-Verify Script cannot deflate it"
        );
        assert_eq!(
            script2_emitted.source(),
            SourceTag::Agent,
            "the Agent source propagates through the deterministic downstream"
        );
    }

    /// Mirrors `onsager_substrate::validate::propagate_max_uncertainty`.
    /// Documented in the test above.
    fn kernel_emit(declared: Provenance, inputs: &[Provenance]) -> Provenance {
        if declared.is_uncertain() {
            return declared;
        }
        if let Some(uncertain) = inputs.iter().copied().find(Provenance::is_uncertain) {
            return Provenance::Uncertain {
                source: uncertain.source(),
            };
        }
        declared
    }

    #[test]
    fn agent_executor_serializes_with_kind_discriminator() {
        // Serializing a `Box<dyn SubstrateExecutor>` writes the
        // typetag `kind` field. The runner is `#[serde(skip)]` and
        // never appears on the wire.
        let exec: Box<dyn SubstrateExecutor> = Box::new(
            AgentExecutor::new("claude-sonnet-4-6", "")
                .with_tools(vec!["edit".into()])
                .with_credential_ref("creds_42"),
        );
        let json = serde_json::to_value(&exec).unwrap();
        assert_eq!(json["kind"], serde_json::json!("agent"));
        assert_eq!(json["model"], serde_json::json!("claude-sonnet-4-6"));
        assert_eq!(json["tools"], serde_json::json!(["edit"]));
        assert_eq!(json["credential_ref"], serde_json::json!("creds_42"));
        assert!(
            json.get("runner").is_none(),
            "runner is #[serde(skip)] — never written"
        );

        // Round-trip: the deserialized AgentExecutor reports the
        // same kind and is callable for declared_provenance even
        // without a runner.
        let roundtrip: Box<dyn SubstrateExecutor> = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.executor_kind(), "agent");
        assert_eq!(
            roundtrip.declared_provenance(&[]),
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
    }

    #[tokio::test]
    async fn agent_dispatchable_through_executor_registry() {
        let mut registry = ExecutorRegistry::new();
        let agent = agent_with_stub("dispatched output");
        registry.register(Arc::new(agent));

        let exec = registry.get("agent").expect("agent should be registered");
        assert_eq!(exec.executor_kind(), "agent");

        let node_id = NodeId::generate();
        let (ctx, _spine) = ctx_with_manifest(node_id, manifest_one_emit());
        let outputs = exec.execute(ctx).await.unwrap();
        let (_id, art) = &outputs.artifacts[0];
        assert!(art.provenance.is_uncertain());
        assert_eq!(art.provenance.source(), SourceTag::Agent);
        assert_eq!(art.produced_by_node, Some(node_id));
    }

    // -----------------------------------------------------------------
    // §4b — widened response is an optimization, manifest is authority.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn manifest_is_authoritative_regardless_of_response_emitted() {
        // A runner that surfaces emitted refs on the response.
        #[derive(Debug)]
        struct EmittingRunner(Vec<EmittedArtifact>);
        #[async_trait]
        impl AgentRunner for EmittingRunner {
            async fn run(&self, _r: AgentRequest) -> Result<AgentResponse, AgentRunError> {
                Ok(AgentResponse {
                    output: "done".into(),
                    emitted: self.0.clone(),
                })
            }
        }

        let manifest = manifest_one_emit();
        let node_id = NodeId::generate();

        // Run A: runner reports the same emit the manifest has.
        let exec_a = AgentExecutor::new("m", "p")
            .with_runner(Arc::new(EmittingRunner(manifest.emitted.clone())));
        let (ctx_a, _) = ctx_with_manifest(node_id, manifest.clone());
        let out_a = exec_a.execute(ctx_a).await.unwrap();

        // Run B: runner reports NOTHING on the response, but the
        // manifest still carries the emit. Behavior must be identical —
        // the executor reads the manifest, not the response.
        let exec_b = AgentExecutor::new("m", "p").with_runner(Arc::new(EmittingRunner(Vec::new())));
        let (ctx_b, _) = ctx_with_manifest(node_id, manifest.clone());
        let out_b = exec_b.execute(ctx_b).await.unwrap();

        assert_eq!(out_a.artifacts.len(), out_b.artifacts.len());
        assert_eq!(out_a.artifacts.len(), 1);
        assert_eq!(
            out_a.artifacts[0].1.versions[0].content_ref,
            out_b.artifacts[0].1.versions[0].content_ref,
        );
    }

    /// Compile-time check: the runtime trait is still object-safe
    /// once `AgentExecutor` is wired in.
    #[test]
    fn agent_executor_trait_object_safe() {
        let _boxed: Box<dyn Executor> = Box::new(AgentExecutor::new("m", "p"));
        let _arced: Arc<dyn Executor> = Arc::new(AgentExecutor::new("m", "p"));
    }

    #[test]
    fn agent_runner_trait_object_safe() {
        let _arced: Arc<dyn AgentRunner> = Arc::new(StubAgentRunner::new("x"));
        let _arced: Arc<dyn AgentRunner> = Arc::new(UnconfiguredRunner);
    }
}
