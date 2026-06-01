//! [`ExecutorContext`] / [`ExecutorOutputs`] — the input and output
//! shapes the runtime hands to / takes back from an executor.
//!
//! The scheduler (RUN-01, #359) builds a context from the upstream
//! edges' resolved artifacts, calls `Executor::execute`, and routes
//! the returned [`ExecutorOutputs`] onto the downstream edges. This
//! crate doesn't host the scheduler — only the data shapes the
//! executor sees.

use std::sync::Arc;

use onsager_artifact::{Artifact, ArtifactId, NodeId};
use onsager_substrate::executor::AgentConfig;
use onsager_substrate::ids::WorkflowId;

use crate::SpineClient;
use crate::scheduler::PlanId;

/// What an executor receives when the runtime invokes it.
///
/// The fields are exactly the runtime context the kernel commits to;
/// future capabilities (cancellation, structured logging, secrets)
/// extend this struct rather than adding new arguments to
/// `Executor::execute`.
pub struct ExecutorContext {
    /// Identifier for the Execution Plan run this dispatch belongs
    /// to. Threaded onto every substrate lifecycle event the executor
    /// emits (`agent.session_*`, `synodic.verdict`) so consumers can
    /// correlate them back to a specific run — without it the spine
    /// envelope's `stream_id` (`plan:<plan_id>:<node_id>`) collapses
    /// across runs.
    pub plan_id: PlanId,
    /// The node whose executor this is — identifies the producer for
    /// downstream `Artifact::produced_by_node` tagging.
    pub node_id: NodeId,
    /// Resolved upstream artifacts on this node's input edges, in
    /// declaration order. The runtime is responsible for matching
    /// these up to the executor's expected input shape.
    pub inputs: Vec<(ArtifactId, Artifact)>,
    /// Port for emitting events and reading other artifacts. Shared
    /// across executors in a run.
    pub spine: Arc<dyn SpineClient>,
    /// `Some(workflow_id)` if the node's substrate-side executor is a
    /// SubWorkflow (its
    /// [`onsager_substrate::executor::Executor::subworkflow_ref`]
    /// returned `Some`); `None` otherwise.
    ///
    /// The dispatch path uses `ExecutorRegistry` lookup keyed on the
    /// executor kind string, which means the registered runtime
    /// instance is shared across every node of that kind — per-node
    /// configuration on the substrate-side executor is invisible to
    /// the runtime instance. For SubWorkflow (EXE-05, #357) the per-
    /// node `workflow_ref` is the whole point of the node, so the
    /// scheduler reads it off the substrate executor and threads it
    /// through here. Other executors ignore the field.
    pub subworkflow_ref: Option<WorkflowId>,
    /// `Some(config)` if the node's substrate-side executor is an Agent
    /// (its [`onsager_substrate::executor::Executor::agent_config`]
    /// returned `Some`); `None` otherwise.
    ///
    /// The agent-config sibling of [`Self::subworkflow_ref`], for the
    /// same reason: dispatch shares one registered `AgentExecutor`
    /// across every agent node, so its per-instance config is invisible
    /// to the runtime instance. The scheduler reads the per-node config
    /// off the substrate executor and threads it through here; the
    /// runtime `AgentExecutor` prefers it over its own fields. Other
    /// executors ignore the field. Issue #534.
    pub agent_config: Option<AgentConfig>,
    /// Plan-scoped workspace session token (spec #536), already
    /// decrypted, shared across every agent node in this plan run. The
    /// `AgentExecutor` copies it onto its [`crate::AgentRequest`] so the
    /// runner injects it as `ONSAGER_SESSION_TOKEN` into the agent's
    /// execution environment — the scheduler-path counterpart of the
    /// chat path's per-session token. `None` when no plan token was
    /// minted (no credential key configured) or for non-agent nodes
    /// that ignore it.
    pub session_token: Option<String>,
}

// Manual `Debug` so the plaintext `session_token` (a PAT) never leaks
// through `{:?}` / log lines. Every other field is shown verbatim;
// `session_token` is redacted to its presence (`<redacted>` / `None`).
impl std::fmt::Debug for ExecutorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutorContext")
            .field("plan_id", &self.plan_id)
            .field("node_id", &self.node_id)
            .field("inputs", &self.inputs)
            .field("spine", &self.spine)
            .field("subworkflow_ref", &self.subworkflow_ref)
            .field("agent_config", &self.agent_config)
            .field(
                "session_token",
                if self.session_token.is_some() {
                    &"<redacted>"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

/// What an executor returns to the runtime.
///
/// Each `(ArtifactId, Artifact)` pair lines up with one of the node's
/// declared output edges, again in declaration order. An empty vec is
/// legal — a side-effecting executor that emits events but produces no
/// artifact returns `ExecutorOutputs::empty()`.
#[derive(Debug, Default)]
pub struct ExecutorOutputs {
    pub artifacts: Vec<(ArtifactId, Artifact)>,
    /// `Some(true)` when the executor produced no artifacts **on
    /// purpose** — an explicit empty delivery, distinguishing it from
    /// "delivered nothing" (`artifacts` empty, field `None`). The
    /// scheduler threads this onto `node.completed.declared_empty`
    /// (spec #520 §4c/§4e). `None` for every executor that doesn't
    /// have a no-output concept.
    pub declared_empty: Option<bool>,
}

impl ExecutorOutputs {
    /// An empty outputs value. Used by side-effect-only executors
    /// (including [`crate::NoOpExecutor`]).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build an outputs value from a single `(id, artifact)` pair.
    pub fn single(id: ArtifactId, artifact: Artifact) -> Self {
        Self {
            artifacts: vec![(id, artifact)],
            declared_empty: None,
        }
    }

    /// An empty outputs value that records the executor declared it
    /// produced nothing on purpose (spec #520 §4c). Surfaces as
    /// `node.completed.declared_empty = Some(true)`.
    pub fn declared_empty() -> Self {
        Self {
            artifacts: Vec::new(),
            declared_empty: Some(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::{MockSpine, dummy_artifact};

    #[test]
    fn outputs_empty_has_no_artifacts() {
        assert!(ExecutorOutputs::empty().artifacts.is_empty());
        assert!(ExecutorOutputs::default().artifacts.is_empty());
    }

    #[test]
    fn outputs_single_carries_one_pair() {
        let art = dummy_artifact();
        let id = art.artifact_id.clone();
        let out = ExecutorOutputs::single(id.clone(), art);
        assert_eq!(out.artifacts.len(), 1);
        assert_eq!(out.artifacts[0].0, id);
    }

    #[test]
    fn context_is_constructible_with_mock_spine() {
        let ctx = ExecutorContext {
            plan_id: PlanId::generate(),
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
            agent_config: None,
            session_token: None,
        };
        assert!(ctx.inputs.is_empty());
        assert!(ctx.subworkflow_ref.is_none());
        assert!(ctx.session_token.is_none());
    }
}
