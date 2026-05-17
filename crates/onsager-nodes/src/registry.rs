//! [`ExecutorRegistry`] — the runtime catalog mapping
//! `executor_kind` → `Arc<dyn Executor>`.
//!
//! Populated at startup, looked up at dispatch time. The registry is
//! the single point of failure for "did you remember to register your
//! executor" (ADR 0012 § Negative consequences); workflows referencing
//! an unregistered kind fail at [`crate::dispatch`] with
//! [`crate::ExecutorError::UnknownKind`].

use std::collections::HashMap;
use std::sync::Arc;

use crate::executor::{Executor, NoOpExecutor};

/// Lookup table for runtime executors, keyed by `executor_kind()`.
///
/// Build one per process at startup, register each concrete executor
/// once, then hand `&self` to the scheduler. Mutable construction is
/// intentional — the registry is finalized before dispatch starts;
/// hot-reload is explicitly out of scope (ADR 0012 § Out of scope).
#[derive(Debug, Default)]
pub struct ExecutorRegistry {
    by_kind: HashMap<&'static str, Arc<dyn Executor>>,
}

impl ExecutorRegistry {
    /// An empty registry. Combine with [`Self::register`] to populate.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-populated with the no-op executor.
    ///
    /// Useful for early-bringup runtimes and tests that exercise the
    /// dispatch machinery before any real executor (EXE-02..06) has
    /// landed.
    pub fn with_noop() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(NoOpExecutor));
        r
    }

    /// Register an executor. Subsequent calls to [`Self::get`] with
    /// the same `executor_kind()` resolve to this instance. A second
    /// registration under the same kind silently replaces the first —
    /// startup wiring controls the order, so "last writer wins"
    /// matches the way registries are built in practice.
    pub fn register(&mut self, executor: Arc<dyn Executor>) {
        let kind = executor.executor_kind();
        self.by_kind.insert(kind, executor);
    }

    /// Look up the runtime executor for a kind, if registered.
    pub fn get(&self, kind: &str) -> Option<Arc<dyn Executor>> {
        self.by_kind.get(kind).cloned()
    }

    /// Iterate over the registered kinds. Order is unspecified.
    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_kind.keys().copied()
    }

    /// Number of registered executors.
    pub fn len(&self) -> usize {
        self.by_kind.len()
    }

    /// Whether the registry has no executors.
    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutorContext;
    use crate::spine::test_support::MockSpine;
    use onsager_artifact::NodeId;

    #[test]
    fn new_registry_is_empty() {
        let r = ExecutorRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.get("noop").is_none());
    }

    #[test]
    fn with_noop_registers_the_noop_executor() {
        let r = ExecutorRegistry::with_noop();
        assert_eq!(r.len(), 1);
        let exec = r.get("noop").expect("noop should be registered");
        assert_eq!(exec.executor_kind(), "noop");
    }

    #[test]
    fn register_overwrites_same_kind() {
        let mut r = ExecutorRegistry::new();
        r.register(Arc::new(NoOpExecutor));
        r.register(Arc::new(NoOpExecutor));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn kinds_lists_registered_executors() {
        let r = ExecutorRegistry::with_noop();
        let kinds: Vec<&'static str> = r.kinds().collect();
        assert_eq!(kinds, vec!["noop"]);
    }

    #[tokio::test]
    async fn noop_dispatched_through_registry_returns_empty_outputs() {
        // Acceptance test from issue #353 verification list:
        // "NoOpExecutor dispatched through the registry returns
        // ExecutorOutputs { artifacts: vec![] }".
        let registry = ExecutorRegistry::with_noop();
        let exec = registry.get("noop").expect("registered");
        let ctx = ExecutorContext {
            node_id: NodeId::generate(),
            inputs: vec![],
            spine: Arc::new(MockSpine::default()),
            subworkflow_ref: None,
        };
        let outputs = exec.execute(ctx).await.unwrap();
        assert!(outputs.artifacts.is_empty());
    }
}
