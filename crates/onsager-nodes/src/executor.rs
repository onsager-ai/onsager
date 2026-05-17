//! The runtime [`Executor`] trait and the [`NoOpExecutor`] reference
//! implementation.
//!
//! See [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md).
//! The static / serializable half of "what a node carries" lives in
//! [`onsager_substrate::Executor`] — this trait is the runtime sibling.
//! Both halves share an `executor_kind()` string: the substrate side
//! uses it as a typetag discriminator, the runtime side uses it as the
//! [`crate::ExecutorRegistry`] lookup key. A workflow's static
//! `executor.executor_kind()` and the registered runtime executor's
//! `executor_kind()` must agree — that's the only contract the
//! kernel relies on to route a node to its behavior.
//!
//! Trait-object safety is load-bearing: the registry stores
//! `Arc<dyn Executor>` and the scheduler dispatches through it. No
//! generic methods, no `where Self: Sized`.

use async_trait::async_trait;
use onsager_artifact::Provenance;

use crate::context::{ExecutorContext, ExecutorOutputs};
use crate::error::ExecutorError;

/// What a node *does* at runtime.
///
/// Implementations live as flat sibling modules in this crate
/// (`script.rs`, `agent.rs`, `verify.rs`, `human.rs`, `subworkflow.rs`)
/// — see ADR 0012 § Decision. Each implementation registers itself
/// with the [`crate::ExecutorRegistry`] at startup so the scheduler
/// can resolve a node's kind to the running instance.
///
/// `declared_provenance` mirrors the substrate trait's contract: a
/// non-Verify executor must return at least the worst input provenance
/// (the kernel's invariant 2 from ADR 0018 enforces this at validate
/// time). Verify (EXE-04) is the only executor allowed to upgrade
/// `Uncertain` inputs to `Deterministic`.
#[async_trait]
pub trait Executor: Send + Sync + std::fmt::Debug {
    /// The wire-format tag for this executor. Must match the
    /// `executor_kind()` of the corresponding
    /// [`onsager_substrate::Executor`] implementation — the registry
    /// uses this string as its lookup key.
    fn executor_kind(&self) -> &'static str;

    /// Provenance this executor claims to produce, given its inputs.
    /// See [`onsager_substrate::Executor::declared_provenance`] for
    /// the contract; the runtime side restates it so an executor that
    /// only implements the runtime trait still answers the question.
    fn declared_provenance(&self, input_provenances: &[Provenance]) -> Provenance;

    /// Run the node. The runtime supplies upstream artifacts and a
    /// spine port; the executor returns the artifacts it produced (or
    /// `ExecutorOutputs::empty()` for side-effect-only kinds).
    async fn execute(&self, ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError>;
}

/// A no-op runtime executor — produces no artifacts, emits no events.
///
/// The runtime counterpart of [`onsager_substrate::NoOpExecutor`].
/// Both register under the kind string `"noop"`, so a workflow whose
/// nodes use the substrate `NoOpExecutor` dispatches correctly the
/// moment the runtime side is registered.
///
/// Use cases:
/// - the registry's default-registered executor, so dispatch compiles
///   and works end-to-end before any real executor (EXE-02..06) lands;
/// - placeholder while a workflow author stubs a node before the real
///   executor exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpExecutor;

#[async_trait]
impl Executor for NoOpExecutor {
    fn executor_kind(&self) -> &'static str {
        "noop"
    }

    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance {
        // Mirror the substrate NoOpExecutor: propagate the worst input
        // (any `Uncertain` wins), default to external-deterministic
        // when there are no inputs at all.
        inputs
            .iter()
            .copied()
            .find(Provenance::is_uncertain)
            .unwrap_or_default()
    }

    async fn execute(&self, _ctx: ExecutorContext) -> Result<ExecutorOutputs, ExecutorError> {
        Ok(ExecutorOutputs::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::{NodeId, SourceTag};
    use std::sync::Arc;

    fn empty_ctx() -> ExecutorContext {
        ExecutorContext {
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        }
    }

    #[tokio::test]
    async fn noop_execute_returns_empty_outputs() {
        let out = NoOpExecutor.execute(empty_ctx()).await.unwrap();
        assert!(out.artifacts.is_empty());
    }

    #[test]
    fn noop_executor_kind_is_noop() {
        assert_eq!(NoOpExecutor.executor_kind(), "noop");
    }

    #[test]
    fn noop_declared_provenance_with_no_inputs_is_external_deterministic() {
        assert_eq!(
            NoOpExecutor.declared_provenance(&[]),
            Provenance::external_deterministic()
        );
    }

    #[test]
    fn noop_declared_provenance_propagates_uncertain_input() {
        let p = NoOpExecutor.declared_provenance(&[
            Provenance::Deterministic {
                source: SourceTag::Script,
            },
            Provenance::Uncertain {
                source: SourceTag::Agent,
            },
        ]);
        assert!(p.is_uncertain());
        assert_eq!(p.source(), SourceTag::Agent);
    }

    /// Compile-time check: the trait is object-safe and `Box<dyn>` /
    /// `Arc<dyn>` storage works without `where Self: Sized` hacks.
    #[test]
    fn executor_trait_is_object_safe() {
        let _boxed: Box<dyn Executor> = Box::new(NoOpExecutor);
        let _arced: Arc<dyn Executor> = Arc::new(NoOpExecutor);
    }
}
