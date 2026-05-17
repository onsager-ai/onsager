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
use onsager_substrate::ids::WorkflowId;

use crate::SpineClient;

/// What an executor receives when the runtime invokes it.
///
/// The fields are exactly the runtime context the kernel commits to;
/// future capabilities (cancellation, structured logging, secrets)
/// extend this struct rather than adding new arguments to
/// `Executor::execute`.
#[derive(Debug)]
pub struct ExecutorContext {
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
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        };
        assert!(ctx.inputs.is_empty());
        assert!(ctx.subworkflow_ref.is_none());
    }
}
