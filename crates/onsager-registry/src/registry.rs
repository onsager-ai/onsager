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
//! 3. Three trait-based plug points: [`ArtifactAdapter`], [`GateEvaluator`],
//!    [`AgentProfile`] — registered by id so that adding a new adapter or
//!    swapping a model is a registry update, not a code change.
//! 4. `workspace_id` on every row; default `"default"`. No auth enforcement
//!    yet, but the column exists from day 1.
//! 5. Registry entries are themselves artifacts, mutated via spine events
//!    (`type.proposed`, `type.approved`, `adapter.registered`, …). The
//!    registry tables are the projection of that event stream.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use onsager_artifact::ArtifactId;

/// Default workspace id used when no explicit workspace is provided.
pub const DEFAULT_WORKSPACE: &str = "default";

/// Actor name recorded on events emitted by the idempotent seed loader.
///
/// Bootstrap termination: the seed loader uses this actor exactly once per
/// workspace. After boot, every registry change goes through gates like any
/// other artifact.
pub const SEED_ACTOR: &str = "seed";

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

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

/// Lifecycle status of a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryStatus {
    /// Proposed but not yet approved. Artifacts cannot use it.
    Proposed,
    /// Approved and in active use.
    Approved,
    /// Kept for audit but no longer used for new artifacts.
    Deprecated,
}

impl RegistryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Deprecated => "deprecated",
        }
    }
}

// ---------------------------------------------------------------------------
// Type definitions
// ---------------------------------------------------------------------------

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

/// A registered artifact type as stored in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredType {
    pub workspace_id: String,
    pub revision: i32,
    pub status: RegistryStatus,
    pub definition: TypeDefinition,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Adapter plug point
// ---------------------------------------------------------------------------

/// An opaque reference to an artifact stored in an external system of record.
///
/// The adapter owns the schema; the engine just passes the string through.
/// Examples: `"gh:onsager-ai/onsager#14"`, `"railway:env/preview-42"`,
/// `"git:tag/v0.4.2"`, `"notion:page/abc"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalRef(pub String);

impl ExternalRef {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExternalRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Material that an adapter fetched from or wrote to the external system.
///
/// The engine stores this as the artifact's `metadata`; no type-specific
/// columns are added to the artifacts table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AdapterMaterial {
    /// The external ref (e.g. GitHub issue URL, PR number) returned by
    /// `create` or the input to `fetch`/`update`.
    pub external_ref: Option<ExternalRef>,
    /// Free-form metadata the adapter wants to persist on the artifact.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Adapter errors are `anyhow` to keep the trait simple; concrete adapters can
/// surface richer errors in their own API.
pub type AdapterResult<T> = anyhow::Result<T>;

/// Binds an artifact type to an external system of record.
///
/// Implementations are registered by id in the `artifact_adapters` table; the
/// engine looks up the concrete adapter by the type definition's `adapter_id`
/// and never imports it directly.
#[async_trait]
pub trait ArtifactAdapter: Send + Sync {
    /// Stable id, e.g. `"github.issue"`, `"github.pr"`, `"railway.env"`.
    fn adapter_id(&self) -> &RegistryId;

    /// Create a new external resource for this artifact.
    /// Returns the external ref + metadata to persist on the artifact.
    async fn create(
        &self,
        artifact_id: &ArtifactId,
        params: &serde_json::Value,
    ) -> AdapterResult<AdapterMaterial>;

    /// Fetch the current state from the external system.
    async fn fetch(&self, external_ref: &ExternalRef) -> AdapterResult<AdapterMaterial>;

    /// Apply an update to the external resource (label change, comment, merge,
    /// tag move, …). Returns the refreshed material.
    async fn update(
        &self,
        external_ref: &ExternalRef,
        patch: &serde_json::Value,
    ) -> AdapterResult<AdapterMaterial>;
}

// ---------------------------------------------------------------------------
// Gate evaluator plug point
// ---------------------------------------------------------------------------

/// Input to a gate evaluator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateContext {
    pub artifact_id: ArtifactId,
    pub type_id: RegistryId,
    pub workspace_id: String,
    /// Free-form context the pipeline wants the evaluator to consider
    /// (ShapingResult, external status, reviewer list, …).
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Verdict returned by a gate evaluator.
///
/// This mirrors the coarser `onsager_spine::factory_event::VerdictSummary` used for
/// event auditing, but carries a reason so the event envelope can be populated
/// directly from the verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GateVerdict {
    Allow {
        #[serde(default)]
        reason: String,
    },
    Deny {
        reason: String,
    },
    /// Needs human (or delegated) approval before the transition can proceed.
    Escalate {
        reason: String,
    },
}

impl GateVerdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}

/// Allow/deny a state transition. Implementations are registered by id and
/// composable via [`CompositeGate`].
#[async_trait]
pub trait GateEvaluator: Send + Sync {
    fn evaluator_id(&self) -> &RegistryId;
    async fn evaluate(&self, ctx: &GateContext) -> AdapterResult<GateVerdict>;
}

/// Compose several [`GateEvaluator`]s via logical AND / OR.
///
/// - `AllOf`: all must allow (deny / escalate short-circuits).
/// - `AnyOf`: first allow wins; otherwise the combined denial is returned.
pub enum CompositeGate<'a> {
    AllOf(&'a [&'a dyn GateEvaluator]),
    AnyOf(&'a [&'a dyn GateEvaluator]),
}

impl<'a> CompositeGate<'a> {
    pub async fn evaluate(&self, ctx: &GateContext) -> AdapterResult<GateVerdict> {
        match self {
            Self::AllOf(gates) => {
                let mut reasons = Vec::new();
                for g in gates.iter() {
                    match g.evaluate(ctx).await? {
                        GateVerdict::Allow { reason } => {
                            if !reason.is_empty() {
                                reasons.push(format!("{}: {}", g.evaluator_id(), reason));
                            }
                        }
                        other => return Ok(other),
                    }
                }
                Ok(GateVerdict::Allow {
                    reason: reasons.join("; "),
                })
            }
            Self::AnyOf(gates) => {
                let mut denials = Vec::new();
                for g in gates.iter() {
                    match g.evaluate(ctx).await? {
                        GateVerdict::Allow { reason } => return Ok(GateVerdict::Allow { reason }),
                        GateVerdict::Deny { reason } => {
                            denials.push(format!("{}: {}", g.evaluator_id(), reason));
                        }
                        GateVerdict::Escalate { reason } => {
                            denials.push(format!("{}: escalate: {}", g.evaluator_id(), reason));
                        }
                    }
                }
                Ok(GateVerdict::Deny {
                    reason: denials.join("; "),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Agent profile plug point
// ---------------------------------------------------------------------------

/// A reusable bundle of agent configuration: role, system prompt, tool set,
/// model. Types reference profiles by id so swapping a model is a registry
/// update, not a code change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub profile_id: RegistryId,
    /// Role label, e.g. `"spec-writer"`, `"implementer"`, `"human"`.
    pub role: String,
    /// The agent's system prompt. `None` means the profile is non-AI
    /// (e.g. `Human`).
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Ids of tools the agent is permitted to use.
    #[serde(default)]
    pub tool_ids: Vec<String>,
    /// Model identifier, e.g. `"claude-opus-4-7"`. `None` for non-AI profiles.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedGate {
        id: RegistryId,
        verdict: GateVerdict,
    }

    #[async_trait]
    impl GateEvaluator for FixedGate {
        fn evaluator_id(&self) -> &RegistryId {
            &self.id
        }
        async fn evaluate(&self, _ctx: &GateContext) -> AdapterResult<GateVerdict> {
            Ok(self.verdict.clone())
        }
    }

    fn ctx() -> GateContext {
        GateContext {
            artifact_id: ArtifactId::new("art_test12345678"),
            type_id: "Spec".into(),
            workspace_id: DEFAULT_WORKSPACE.into(),
            payload: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn all_of_requires_every_allow() {
        let a = FixedGate {
            id: "A".into(),
            verdict: GateVerdict::Allow {
                reason: String::new(),
            },
        };
        let b = FixedGate {
            id: "B".into(),
            verdict: GateVerdict::Deny {
                reason: "nope".into(),
            },
        };
        let gates: Vec<&dyn GateEvaluator> = vec![&a, &b];
        let v = CompositeGate::AllOf(&gates).evaluate(&ctx()).await.unwrap();
        assert!(matches!(v, GateVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn any_of_returns_first_allow() {
        let a = FixedGate {
            id: "A".into(),
            verdict: GateVerdict::Deny {
                reason: "first".into(),
            },
        };
        let b = FixedGate {
            id: "B".into(),
            verdict: GateVerdict::Allow {
                reason: "second".into(),
            },
        };
        let gates: Vec<&dyn GateEvaluator> = vec![&a, &b];
        let v = CompositeGate::AnyOf(&gates).evaluate(&ctx()).await.unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn any_of_denies_with_aggregated_reasons() {
        let a = FixedGate {
            id: "A".into(),
            verdict: GateVerdict::Deny { reason: "a".into() },
        };
        let b = FixedGate {
            id: "B".into(),
            verdict: GateVerdict::Deny { reason: "b".into() },
        };
        let gates: Vec<&dyn GateEvaluator> = vec![&a, &b];
        let v = CompositeGate::AnyOf(&gates).evaluate(&ctx()).await.unwrap();
        match v {
            GateVerdict::Deny { reason } => {
                assert!(reason.contains("A: a"));
                assert!(reason.contains("B: b"));
            }
            _ => panic!("expected deny, got {v:?}"),
        }
    }

    #[test]
    fn type_definition_serde_roundtrip() {
        let def = TypeDefinition {
            type_id: "Issue".into(),
            description: "Engineering issue artifact.".into(),
            adapter_id: "github.issue".into(),
            gate_ids: vec!["ReviewApproved".into()],
            producer_profile_id: Some("spec-writer".into()),
            config: serde_json::json!({"label": "spec"}),
            intrinsic_schema: serde_json::json!({"type": "object"}),
            merge_rule: MergeRule::Overwrite,
        };
        let yaml = serde_yaml::to_string(&def).unwrap();
        let back: TypeDefinition = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn type_definition_defaults_merge_rule_to_overwrite() {
        let yaml = r#"
type_id: Simple
adapter_id: github.issue
"#;
        let def: TypeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.merge_rule, MergeRule::Overwrite);
        assert!(def.intrinsic_schema.is_null());
    }

    #[test]
    fn merge_rule_deep_merge_serializes_snake_case() {
        let json = serde_json::to_string(&MergeRule::DeepMerge).unwrap();
        assert_eq!(json, r#""deep_merge""#);
        let back: MergeRule = serde_json::from_str(r#""merge_by_key""#).unwrap();
        assert_eq!(back, MergeRule::MergeByKey);
    }

    #[test]
    fn registry_status_strings() {
        assert_eq!(RegistryStatus::Approved.as_str(), "approved");
        assert_eq!(RegistryStatus::Proposed.as_str(), "proposed");
        assert_eq!(RegistryStatus::Deprecated.as_str(), "deprecated");
    }
}
