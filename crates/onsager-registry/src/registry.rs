//! Factory pipeline registry — type catalog, adapters, gate evaluators, agent
//! profiles.
//!
//! See GitHub issue #14 for the design. The five founding decisions are:
//!
//! 1. Types are data, not code. The engine never matches on type variants;
//!    everything flows through an id + JSON definition.
//! 2. Artifacts are thin handles: `(id, type_id, adapter_id, external_ref,
//!    state, metadata, lineage, workspace_id)`. Content lives in the external
//!    system (GitHub issue, Railway env, git tag, …).
//! 3. The surviving plug point: the code-defined workflow-kind catalog
//!    [`AgentProfile`] — registered by id so that adding a new adapter or
//!    swapping a model is a registry update, not a code change.
//! 4. `workspace_id` on every row; default `"default"`. No auth enforcement
//!    yet, but the column exists from day 1.
//! 5. Registry entries are themselves artifacts, mutated via spine events
//!    (`type.proposed`, `type.approved`, `adapter.registered`, …). The
//!    registry tables are the projection of that event stream.

use serde::{Deserialize, Serialize};

/// Stable identifier for a registered entry (type, adapter, evaluator, profile).
///
/// Registry ids are human-readable strings (e.g. `"Spec"`, `"github.pr"`,
/// `"CiGreen"`), not ULIDs. The primary key is the pair `(workspace_id, id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistryId(pub String);

impl RegistryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RegistryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for RegistryId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for RegistryId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Per-kind merge rule. Declares how partial updates to an artifact of this
/// kind combine — the canonical example is a `PullRequest`, which accumulates
/// commits, checks, and reviews as gates complete (see issue #103). Merge
/// rules are a first-class registry concept: every registered
/// [`TypeDefinition`] declares one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeRule {
    /// Latest write wins. The default for simple value kinds.
    #[default]
    Overwrite,
    /// Treat the artifact as a map keyed by an identity field; merge by key.
    MergeByKey,
    /// List concatenation — callers guarantee idempotency on exact element.
    Append,
    /// Recursive per-field merge. Used by `PullRequest` to fold in partial
    /// updates (commits: append, checks/reviews: merge-by-key, merged: overwrite).
    DeepMerge,
}

/// Definition of an artifact type — the shape of every `Issue`, `PR`,
/// `TestableEnvironment`, etc.
///
/// Types are data: no variants in the engine. Adding a new type is a registry
/// insert (a `type.proposed` event followed by `type.approved`). The engine
/// reads `adapter_id`, `gate_ids`, `producer_profile_id`, `intrinsic_schema`,
/// and `merge_rule` and looks up the corresponding registered implementations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefinition {
    /// Stable id, e.g. `"Issue"`, `"PR"`.
    pub type_id: RegistryId,
    /// Human-readable description (one sentence).
    #[serde(default)]
    pub description: String,
    /// Id of the [`ArtifactAdapter`] that owns the external ref for this type.
    pub adapter_id: RegistryId,
    /// Gate evaluators consulted on state transitions; composed with AllOf.
    #[serde(default)]
    pub gate_ids: Vec<RegistryId>,
    /// The agent profile that produces artifacts of this type (if any).
    #[serde(default)]
    pub producer_profile_id: Option<RegistryId>,
    /// Arbitrary type-specific configuration (labels, templates, …).
    #[serde(default)]
    pub config: serde_json::Value,
    /// JSON-Schema fragment describing the intrinsic fields on artifacts of
    /// this kind (issue #102). Optional — kinds that only carry a reference
    /// (e.g. `Issue`) can leave it `Null`.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub intrinsic_schema: serde_json::Value,
    /// How partial updates to an artifact of this kind combine.
    #[serde(default)]
    pub merge_rule: MergeRule,
}

/// Adapter errors are `anyhow` to keep the trait simple; concrete adapters can
/// surface richer errors in their own API.
pub type AdapterResult<T> = anyhow::Result<T>;
