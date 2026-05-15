//! Provenance — substrate first-class type tracking whether an
//! artifact was produced deterministically or by an uncertain process.
//!
//! See [ADR 0010](../../../../docs/adr/0010-provenance-as-substrate-first-class.md).
//!
//! The kernel question this answers: given an artifact, what must be
//! true for a downstream consumer to trust it as deterministic? The
//! two-valued `Provenance` enum is the decidable shape; a Verify
//! executor (EXE-04) is the only kernel-recognized upgrade path from
//! `Uncertain` to `Deterministic`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The producer kind that emitted an artifact.
///
/// Wire form is snake_case: `"agent"`, `"script"`, `"external"`,
/// `"human"`, `"composed"`. Used both inside [`Provenance`] and as a
/// standalone tag on events that carry producer identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTag {
    /// LLM / model output.
    Agent,
    /// Deterministic code (sandboxed).
    Script,
    /// Upstream system (GitHub, etc.).
    External,
    /// Human edit.
    Human,
    /// Derived from multiple parents (e.g., a composed/merged artifact).
    Composed,
}

impl fmt::Display for SourceTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SourceTag::Agent => "agent",
            SourceTag::Script => "script",
            SourceTag::External => "external",
            SourceTag::Human => "human",
            SourceTag::Composed => "composed",
        };
        f.write_str(s)
    }
}

/// The kernel-level trust classification of an artifact.
///
/// Serialized as a tagged enum with `kind` discriminator and a
/// `source` payload:
///
/// ```json
/// {"kind": "deterministic", "source": "external"}
/// {"kind": "uncertain",     "source": "agent"}
/// ```
///
/// Propagation rule (ADR 0010, invariant 2): a node's emitted
/// provenance is the maximum uncertainty of its declared output
/// provenance and all input provenances. `Uncertain` is contagious;
/// only a Verify executor (EXE-04) may upgrade an `Uncertain` input
/// to `Deterministic` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Produced by a process whose output is reproducible from its
    /// inputs (pinned script, external system-of-record, or a Verify
    /// executor's certification).
    Deterministic { source: SourceTag },
    /// Produced by a process whose output is not reproducible from its
    /// inputs (LLM completion, human edit, composed from `Uncertain`
    /// parents).
    Uncertain { source: SourceTag },
}

impl Provenance {
    /// The default provenance assigned to artifacts that predate the
    /// substrate's provenance field — externally-ingested rows whose
    /// origin is the upstream system-of-record.
    pub fn external_deterministic() -> Self {
        Provenance::Deterministic {
            source: SourceTag::External,
        }
    }

    /// The producer tag inside this provenance, regardless of variant.
    pub fn source(&self) -> SourceTag {
        match self {
            Provenance::Deterministic { source } | Provenance::Uncertain { source } => *source,
        }
    }

    /// Whether this provenance is `Uncertain`. Convenience for the
    /// propagation rule — `is_uncertain` short-circuits the max-of-
    /// inputs check at edge validation time.
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Provenance::Uncertain { .. })
    }
}

impl Default for Provenance {
    fn default() -> Self {
        Provenance::external_deterministic()
    }
}

// ---------------------------------------------------------------------------
// NodeId
// ---------------------------------------------------------------------------

/// Identifier of a node in an Execution Plan.
///
/// A UUID v4 newtype; the full `Node` struct lands in SUB-02 (#349).
/// Held here so `Artifact::produced_by_node` can be typed before the
/// node module exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(uuid::Uuid);

impl NodeId {
    /// Wrap an existing UUID.
    pub fn new(id: uuid::Uuid) -> Self {
        Self(id)
    }

    /// Generate a fresh v4 UUID.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// The wrapped UUID.
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<uuid::Uuid> for NodeId {
    fn from(id: uuid::Uuid) -> Self {
        Self(id)
    }
}

impl From<NodeId> for uuid::Uuid {
    fn from(id: NodeId) -> uuid::Uuid {
        id.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_serde_deterministic() {
        let p = Provenance::Deterministic {
            source: SourceTag::External,
        };
        let json = serde_json::to_value(p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "deterministic", "source": "external"})
        );
        let roundtrip: Provenance = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, p);
    }

    #[test]
    fn provenance_serde_uncertain() {
        let p = Provenance::Uncertain {
            source: SourceTag::Agent,
        };
        let json = serde_json::to_value(p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "uncertain", "source": "agent"})
        );
        let roundtrip: Provenance = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, p);
    }

    #[test]
    fn source_tag_all_variants_serialize_snake_case() {
        for (tag, expected) in [
            (SourceTag::Agent, "agent"),
            (SourceTag::Script, "script"),
            (SourceTag::External, "external"),
            (SourceTag::Human, "human"),
            (SourceTag::Composed, "composed"),
        ] {
            let json = serde_json::to_string(&tag).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let roundtrip: SourceTag = serde_json::from_str(&json).unwrap();
            assert_eq!(roundtrip, tag);
        }
    }

    #[test]
    fn provenance_default_is_external_deterministic() {
        assert_eq!(
            Provenance::default(),
            Provenance::Deterministic {
                source: SourceTag::External
            }
        );
    }

    #[test]
    fn is_uncertain_classifies_variants() {
        assert!(
            !Provenance::Deterministic {
                source: SourceTag::Script
            }
            .is_uncertain()
        );
        assert!(
            Provenance::Uncertain {
                source: SourceTag::Agent
            }
            .is_uncertain()
        );
    }

    #[test]
    fn source_accessor_returns_inner_tag() {
        assert_eq!(
            Provenance::Deterministic {
                source: SourceTag::Script
            }
            .source(),
            SourceTag::Script
        );
        assert_eq!(
            Provenance::Uncertain {
                source: SourceTag::Composed
            }
            .source(),
            SourceTag::Composed
        );
    }

    #[test]
    fn node_id_serde_transparent_uuid() {
        let raw = uuid::Uuid::new_v4();
        let id = NodeId::new(raw);
        let json = serde_json::to_value(id).unwrap();
        // Transparent — wire form is the bare UUID string.
        assert_eq!(json, serde_json::json!(raw.to_string()));
        let roundtrip: NodeId = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, id);
        assert_eq!(id.as_uuid(), raw);
    }

    #[test]
    fn node_id_generate_unique() {
        let a = NodeId::generate();
        let b = NodeId::generate();
        assert_ne!(a, b);
    }
}
