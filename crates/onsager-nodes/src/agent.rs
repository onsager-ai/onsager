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
use onsager_artifact::{Artifact, Kind, Provenance, SourceTag};
use onsager_substrate::events as se;
use onsager_substrate::executor::Executor as SubstrateExecutor;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::executor::Executor;

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
}

/// Response returned by an [`AgentRunner`].
///
/// The agent session's final assistant text is the only piece the
/// executor commits to today — tool-call traces and intermediate
/// messages are an observability concern, not part of the artifact
/// the executor emits.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub output: String,
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
        let request = AgentRequest {
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            tools: self.tools.clone(),
            credential_ref: self.credential_ref.clone(),
            user_prompt: render_user_prompt(&ctx),
        };

        // RUN-02 (#360) lifecycle events. `plan_id` comes off
        // `ExecutorContext`, populated by the scheduler so the
        // spine envelope's `stream_id` (`plan:<plan>:<node>`)
        // correlates verdicts back to a specific run.
        let plan_id = ctx.plan_id.as_str().to_string();
        let session_id = Uuid::new_v4().to_string();
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
                plan_id,
                node_id: ctx.node_id,
                session_id,
                // Token usage is not surfaced through `AgentResponse`
                // yet; the live runner will fill it in via a runner-
                // side hook in a follow-up. Leaving `None` keeps the
                // budget consumer honest — "not reported" ≠ "zero".
                token_usage: None,
            },
        )
        .await;

        let mut artifact = Artifact::new(
            Kind::Document,
            "agent-output",
            "agent",
            ctx.node_id.to_string(),
            Vec::new(),
        );
        artifact.provenance = Executor::declared_provenance(self, &[]);
        artifact.produced_by_node = Some(ctx.node_id);
        // The response body lives on an `ArtifactVersion`; full
        // version persistence (content-ref, change summary) lands
        // with the scheduler (RUN-01, #359). For now, route the
        // body through the artifact's `name` slot so the test
        // harness can assert it.
        artifact.name = response.output;

        let artifact_id = artifact.artifact_id.clone();
        Ok(ExecutorOutputs::single(artifact_id, artifact))
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ExecutorRegistry;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::{ArtifactId, NodeId};

    fn agent_with_stub(output: &str) -> AgentExecutor {
        AgentExecutor::new("claude-sonnet-4-6", "you are helpful")
            .with_runner(Arc::new(StubAgentRunner::new(output)))
    }

    fn empty_ctx() -> ExecutorContext {
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

    #[tokio::test]
    async fn execute_returns_uncertain_agent_artifact() {
        let exec = agent_with_stub("the agent said hello");
        let node_id = NodeId::generate();
        let ctx = ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id,
            inputs: Vec::new(),
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        };

        let outputs = exec.execute(ctx).await.unwrap();
        assert_eq!(outputs.artifacts.len(), 1);
        let (_id, art) = &outputs.artifacts[0];
        assert_eq!(
            art.provenance,
            Provenance::Uncertain {
                source: SourceTag::Agent,
            }
        );
        assert_eq!(art.produced_by_node, Some(node_id));
        // Stubbed runner output flows through.
        assert_eq!(art.name, "the agent said hello");
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
        let ctx = ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id: NodeId::generate(),
            inputs: vec![(input_id.clone(), input_art)],
            spine: Arc::new(MockSpine::default()),
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
        let ctx = ExecutorContext {
            plan_id: crate::scheduler::PlanId::generate(),
            node_id,
            inputs: Vec::new(),
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        };
        let outputs = exec.execute(ctx).await.unwrap();
        let (_id, art) = &outputs.artifacts[0];
        assert!(art.provenance.is_uncertain());
        assert_eq!(art.provenance.source(), SourceTag::Agent);
        assert_eq!(art.produced_by_node, Some(node_id));
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
