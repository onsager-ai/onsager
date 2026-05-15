//! # onsager-substrate
//!
//! Kernel data model for the 0.2 substrate: the `Workflow` template
//! (a node + edge graph), the `Executor` trait that every node carries,
//! and the UUID newtypes that identify them.
//!
//! See:
//! - [ADR 0009](../../../docs/adr/0009-three-layer-pipeline.md) — the
//!   Spec Plan / Workflow / Execution Plan three-layer pipeline.
//! - [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//!   — why nodes carry `Box<dyn Executor>` instead of a `NodeKind` enum.
//! - [ADR 0018](../../../docs/adr/0018-five-kernel-invariants.md) — the
//!   five static validators that operate on the types defined here
//!   (lands in SUB-03, #350).
//!
//! This crate is intentionally pure data + a trait stub: no async, no
//! database, no spine. Downstream crates (`onsager-nodes`, the Plan
//! Compiler, the scheduler) layer behavior on top of these types.

pub mod executor;
pub mod ids;
pub mod workflow;

pub use executor::*;
pub use ids::*;
pub use workflow::*;

// Re-exports for downstream convenience — anyone working with a
// substrate `Workflow` will reach for provenance + artifact identity in
// the same breath.
pub use onsager_artifact::{ArtifactId, NodeId, Provenance, SourceTag};
