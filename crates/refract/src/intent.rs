//! Intent — the input a Refract decomposer expands into artifacts.

use serde::{Deserialize, Serialize};

/// Opaque unique identifier for an intent. Stable across the full
/// `intent_submitted` → `decomposed` / `failed` causal chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IntentId(String);

impl IntentId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Generate a fresh id with an `"int_"` prefix and a ULID body. ULID is
    /// sortable so the spine's default `ORDER BY id DESC` lines up with
    /// submission order when there are many intents inflight.
    pub fn generate() -> Self {
        Self(format!("int_{}", ulid::Ulid::new()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IntentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A high-level unit of work filed against the factory. An intent is
/// decomposed into zero or more artifacts by the matching [`Decomposer`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub id: IntentId,
    /// Stable identifier routed to a [`Decomposer`] (e.g. `"file_migration"`).
    pub intent_class: String,
    /// Free-form human-readable description.
    pub description: String,
    /// Submitter identity — typically a user id or agent name.
    pub submitter: String,
    /// Class-specific payload. The matching decomposer interprets it.
    pub payload: serde_json::Value,
}

impl Intent {
    pub fn new(
        intent_class: impl Into<String>,
        description: impl Into<String>,
        submitter: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: IntentId::generate(),
            intent_class: intent_class.into(),
            description: description.into(),
            submitter: submitter.into(),
            payload,
        }
    }
}
