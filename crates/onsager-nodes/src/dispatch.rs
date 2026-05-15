//! End-to-end dispatch helper — given a substrate [`Node`] and an
//! [`ExecutorRegistry`], find the runtime executor for the node's
//! kind and run it.
//!
//! Future scheduler work (RUN-01, #359) will wrap this with edge
//! resolution, output routing, and provenance propagation. The bare
//! helper exists today so dispatch is testable in isolation and so
//! `cargo build -p onsager-nodes` exercises both the substrate
//! dependency and the runtime registry as a single pipeline.

use onsager_substrate::workflow::Node;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;
use crate::registry::ExecutorRegistry;

/// Resolve `node.executor.executor_kind()` through `registry` and run
/// it. Returns [`ExecutorError::UnknownKind`] if no runtime executor
/// is registered for the node's kind string.
pub async fn dispatch(
    registry: &ExecutorRegistry,
    node: &Node,
    ctx: ExecutorContext,
) -> Result<ExecutorOutputs, ExecutorError> {
    let kind = node.executor.executor_kind();
    let runtime = registry
        .get(kind)
        .ok_or_else(|| ExecutorError::UnknownKind(kind.to_string()))?;
    runtime.execute(ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::NodeId;
    use onsager_substrate::executor::NoOpExecutor as SubstrateNoOp;
    use onsager_substrate::workflow::Node;
    use std::sync::Arc;

    fn noop_node() -> Node {
        Node {
            id: NodeId::generate(),
            executor: Box::new(SubstrateNoOp),
            inputs: vec![],
            outputs: vec![],
        }
    }

    fn empty_ctx() -> ExecutorContext {
        ExecutorContext {
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
        }
    }

    #[tokio::test]
    async fn dispatch_resolves_substrate_node_to_runtime_executor() {
        // The substrate NoOp and the runtime NoOp share the kind
        // string "noop" — that's the entire dispatch contract.
        let registry = ExecutorRegistry::with_noop();
        let node = noop_node();
        let outputs = dispatch(&registry, &node, empty_ctx()).await.unwrap();
        assert!(outputs.artifacts.is_empty());
    }

    #[tokio::test]
    async fn dispatch_unknown_kind_returns_unknown_kind_error() {
        // Empty registry — the node's "noop" kind has no runtime.
        let registry = ExecutorRegistry::new();
        let node = noop_node();
        let err = dispatch(&registry, &node, empty_ctx()).await.unwrap_err();
        match err {
            ExecutorError::UnknownKind(kind) => assert_eq!(kind, "noop"),
            other => panic!("expected UnknownKind, got {other:?}"),
        }
    }
}
