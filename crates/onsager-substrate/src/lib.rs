//! # onsager-substrate
//!
//! Kernel data model for the 0.2 substrate: the `Workflow` template
//! (a node + edge graph), the `Executor` trait that every node carries,
//! the UUID newtypes that identify them, and the
//! [`WorkflowLibrary`](workflow_library::WorkflowLibrary) catalog
//! resolving spec kind → workflow.
//!
//! See:
//! - [ADR 0009](../../../docs/adr/0009-three-layer-pipeline.md) — the
//!   Spec Plan / Workflow / Execution Plan three-layer pipeline.
//! - [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//!   — why nodes carry `Box<dyn Executor>` instead of a `NodeKind` enum.
//! - [ADR 0016](../../../docs/adr/0016-workflow-library-n-isomorphic-islands.md)
//!   — the Workflow Library is a flat catalog: one active `Workflow`
//!   per spec kind. The persistence layer lives in [`workflow_library`]
//!   (SUB-04, #351).
//! - [ADR 0018](../../../docs/adr/0018-five-kernel-invariants.md) — the
//!   five static validators that operate on the types defined here
//!   (lands in SUB-03, #350).
//!
//! The kernel data types ([`Workflow`], [`Node`], [`Edge`], the
//! [`Executor`] trait) stay pure data — no async, no database, no
//! spine. The [`workflow_library`] module is the one exception: the
//! library is `Workflow`-content's persistence layer, and SUB-04
//! (#351) gives it a sqlx-backed implementation so the Plan Compiler
//! (SUB-05, #352) can resolve `kind → Workflow` at runtime.

pub mod executor;
pub mod ids;
pub mod workflow;
pub mod workflow_library;

pub use executor::*;
pub use ids::*;
pub use workflow::*;
pub use workflow_library::*;

// Re-exports for downstream convenience — anyone working with a
// substrate `Workflow` will reach for provenance + artifact identity in
// the same breath.
pub use onsager_artifact::{ArtifactId, NodeId, Provenance, SourceTag};
