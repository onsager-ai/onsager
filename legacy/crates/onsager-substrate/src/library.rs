//! `WorkflowLibrary` — the trait the validator uses to resolve
//! [`crate::executor::Executor::subworkflow_ref`] references.
//!
//! The full library implementation (registration, versioning,
//! `spec_kind` indexing) lands in SUB-04 (#351). Invariant 4 of
//! ADR 0018 only needs the lookup half — given a [`WorkflowId`],
//! is there a registered [`Workflow`] for it, and what does it
//! contain? — so the validator depends on this trait, not the
//! eventual concrete type.

use crate::ids::WorkflowId;
use crate::workflow::Workflow;

/// Lookup surface the validator and Plan Compiler share for the
/// workflow library.
///
/// Implementors must return a stable reference for the lifetime of
/// the lookup; the validator holds the borrow only as long as it
/// needs to walk the workflow.
///
/// Two lookup paths are exposed:
/// - [`WorkflowLibrary::get`] resolves an internal `WorkflowId` —
///   the validator uses it for invariant 4 (SubWorkflow refs).
/// - [`WorkflowLibrary::by_kind`] resolves a Spec Plan `kind` string
///   — the Plan Compiler uses it to instantiate one workflow per
///   `SpecRef` (ADR 0017 step 1). Defaults to `None` so existing
///   validator-only impls (e.g. the unit type, fixture libraries)
///   continue to compile; compile against such an impl will fail
///   with `MissingKind` as soon as a real spec is offered, which is
///   the correct behavior.
pub trait WorkflowLibrary {
    /// Fetch a workflow by id, or `None` if no workflow is
    /// registered under that id.
    fn get(&self, id: WorkflowId) -> Option<&Workflow>;

    /// Fetch the latest workflow registered under `spec_kind`. The
    /// Plan Compiler treats `None` as a hard `CompileError::MissingKind`.
    fn by_kind(&self, _spec_kind: &str) -> Option<&Workflow> {
        None
    }
}

/// Empty library — every lookup returns `None`.
///
/// Useful as the default for callers (and tests) that do not yet
/// have a populated library. A workflow that contains a SubWorkflow
/// executor will fail invariant 4 against this library, which is
/// the correct behavior — an unresolved reference is a violation.
impl WorkflowLibrary for () {
    fn get(&self, _id: WorkflowId) -> Option<&Workflow> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A trivial in-memory library used by the validator's tests.
    struct MapLibrary(HashMap<WorkflowId, Workflow>);

    impl WorkflowLibrary for MapLibrary {
        fn get(&self, id: WorkflowId) -> Option<&Workflow> {
            self.0.get(&id)
        }
    }

    #[test]
    fn unit_impl_resolves_nothing() {
        let id = WorkflowId::generate();
        assert!(<() as WorkflowLibrary>::get(&(), id).is_none());
    }

    #[test]
    fn map_impl_resolves_registered_id() {
        let id = WorkflowId::generate();
        let mut map = HashMap::new();
        map.insert(id, Workflow::default());
        let lib = MapLibrary(map);
        assert!(lib.get(id).is_some());
        assert!(lib.get(WorkflowId::generate()).is_none());
    }
}
