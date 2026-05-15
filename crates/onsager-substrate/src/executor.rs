//! The `Executor` trait — how nodes describe their behavior.
//!
//! Per [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md),
//! nodes do not carry a `NodeKind` enum. They carry an
//! `executor: Box<dyn Executor>` and the trait's `executor_kind()`
//! string is the discriminator.
//!
//! This issue (SUB-02, #349) lands the trait *stub* — only the methods
//! the kernel needs to validate a workflow before running it:
//!
//! - [`Executor::executor_kind`] — the wire-format tag (also used by
//!   the kernel to special-case Verify per ADR 0010 / invariant 2).
//! - [`Executor::declared_provenance`] — what provenance this executor
//!   *claims* to produce given its input provenances. Invariant 2
//!   (ADR 0018) tightens this at validate time: for non-Verify
//!   executors, the actual emitted provenance is the max-uncertainty
//!   of `declared_provenance` and all inputs.
//!
//! The `async fn execute(..)` half of the trait lands in EXE-01
//! (#353), in the separate `onsager-nodes` crate — see
//! [ADR 0012](../../../docs/adr/0012-executor-catalog-replaces-nodekind.md)
//! § "Adoption checklist".
//!
//! # Serialization
//!
//! `Box<dyn Executor>` is serialized via [`typetag`]. Every implementor
//! attaches a `#[typetag::serde(name = "...")]` annotation that becomes
//! the `kind` field on the wire:
//!
//! ```json
//! {"kind": "noop"}
//! {"kind": "script", "source_ref": "...", "checksum": "..."}
//! ```
//!
//! The implementor crate must be linked into the binary at runtime; the
//! substrate alone only registers [`NoOpExecutor`] (below). Production
//! executors live in `onsager-nodes` (EXE-02..06) and register
//! themselves the same way.

use onsager_artifact::{Provenance, SourceTag};
use serde::{Deserialize, Serialize};

/// What a node does when the scheduler reaches it.
///
/// Object-safe by design: no generic methods, no `Self: Sized` clauses.
/// `Box<dyn Executor>` is the storage form nodes hold and the wire
/// form serializes through [`typetag`]'s `tag = "kind"` adjacency.
#[typetag::serde(tag = "kind")]
pub trait Executor: std::fmt::Debug + Send + Sync {
    /// Stable wire-format tag for this executor. Must match the
    /// `#[typetag::serde(name = "...")]` on the impl block so
    /// `executor.executor_kind()` round-trips through serde.
    ///
    /// This string is also how the kernel identifies the Verify
    /// executor at validate time (ADR 0010): a node whose
    /// `executor_kind()` is `"verify"` is the only kind allowed to
    /// upgrade `Uncertain` inputs to a `Deterministic` output.
    fn executor_kind(&self) -> &'static str;

    /// The provenance this executor claims to produce, given the
    /// provenances flowing in on its input edges.
    ///
    /// Invariant 2 (ADR 0018) constrains the *actual* provenance: for
    /// non-Verify executors it is the max-uncertainty of this
    /// declared value and all `inputs`. Implementors that do not
    /// upgrade provenance (i.e. everything except Verify) typically
    /// return the max-uncertainty of `inputs` themselves; Verify
    /// returns `Deterministic`.
    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance;
}

/// A no-op executor — declares deterministic output, performs nothing.
///
/// Useful as:
/// - the default executor on hand-written workflow fixtures (this
///   crate's serde round-trip test uses it), and
/// - a placeholder while authors stub out a workflow before the real
///   executor lands.
///
/// On the wire: `{"kind": "noop"}`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoOpExecutor;

#[typetag::serde(name = "noop")]
impl Executor for NoOpExecutor {
    fn executor_kind(&self) -> &'static str {
        "noop"
    }

    /// A no-op preserves the worst input provenance — it neither
    /// degrades nor upgrades. With no inputs, it declares the same
    /// default that fresh externally-ingested artifacts carry
    /// (`Deterministic { source: External }`).
    fn declared_provenance(&self, inputs: &[Provenance]) -> Provenance {
        inputs
            .iter()
            .copied()
            .find(Provenance::is_uncertain)
            .unwrap_or(Provenance::Deterministic {
                source: SourceTag::External,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::SourceTag;

    #[test]
    fn noop_executor_kind() {
        let exec = NoOpExecutor;
        assert_eq!(exec.executor_kind(), "noop");
    }

    #[test]
    fn noop_declared_provenance_with_no_inputs() {
        let p = NoOpExecutor.declared_provenance(&[]);
        assert_eq!(p, Provenance::external_deterministic());
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

    #[test]
    fn noop_executor_roundtrips_as_trait_object() {
        let exec: Box<dyn Executor> = Box::new(NoOpExecutor);
        let json = serde_json::to_value(&exec).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "noop"}));

        let roundtrip: Box<dyn Executor> = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.executor_kind(), "noop");
    }

    #[test]
    fn unknown_executor_kind_fails_to_deserialize() {
        let json = serde_json::json!({"kind": "no-such-executor"});
        let result: Result<Box<dyn Executor>, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "deserializing an unregistered kind should fail"
        );
    }
}
