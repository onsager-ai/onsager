//! UUID newtype identifiers for substrate entities.
//!
//! `NodeId` lives in `onsager-artifact` (it landed alongside
//! `Provenance` in SUB-01, #348, so `Artifact::produced_by_node` could
//! be typed before this crate existed). It is re-exported from the
//! crate root.
//!
//! `WorkflowId` and `EdgeId` are defined here — the `Workflow` and
//! `Edge` types they identify also live in this crate.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! uuid_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(uuid::Uuid);

        impl $name {
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

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<uuid::Uuid> for $name {
            fn from(id: uuid::Uuid) -> Self {
                Self(id)
            }
        }

        impl From<$name> for uuid::Uuid {
            fn from(id: $name) -> uuid::Uuid {
                id.0
            }
        }
    };
}

uuid_newtype! {
    /// Identifier of a `Workflow` template in the Workflow Library.
    ///
    /// Assigned by the library on registration (SUB-04, #351); stable
    /// across all runs of that workflow version.
    WorkflowId
}

uuid_newtype! {
    /// Identifier of an `Edge` inside a `Workflow`.
    ///
    /// Edge identity is local to its workflow — two workflows may
    /// reuse the same `EdgeId` value without conflict because no
    /// kernel operation looks up edges across workflow boundaries.
    EdgeId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_id_serde_transparent_uuid() {
        let raw = uuid::Uuid::new_v4();
        let id = WorkflowId::new(raw);
        let json = serde_json::to_value(id).unwrap();
        assert_eq!(json, serde_json::json!(raw.to_string()));
        let roundtrip: WorkflowId = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, id);
        assert_eq!(id.as_uuid(), raw);
    }

    #[test]
    fn edge_id_serde_transparent_uuid() {
        let raw = uuid::Uuid::new_v4();
        let id = EdgeId::new(raw);
        let json = serde_json::to_value(id).unwrap();
        assert_eq!(json, serde_json::json!(raw.to_string()));
        let roundtrip: EdgeId = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, id);
    }

    #[test]
    fn generate_yields_unique_ids() {
        assert_ne!(WorkflowId::generate(), WorkflowId::generate());
        assert_ne!(EdgeId::generate(), EdgeId::generate());
    }

    #[test]
    fn uuid_conversions_round_trip() {
        let raw = uuid::Uuid::new_v4();
        let id: WorkflowId = raw.into();
        let back: uuid::Uuid = id.into();
        assert_eq!(back, raw);
    }
}
