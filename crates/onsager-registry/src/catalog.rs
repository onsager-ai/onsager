//! First registered catalog — the engineering pipeline (Issue → PR).
//!
//! Per issue #14: the engineering catalog is **not** part of the seed. It
//! goes through the registry just like any other artifact, via
//! [`RegistryStore::propose_type`] + [`RegistryStore::approve_type`]. The
//! base seed (`seeds/base.yaml`) establishes the recursive base; this module
//! is the first thing layered on top.
//!
//! Calling [`register_engineering_catalog`] is idempotent: rerunning it is a
//! no-op (the propose step conflicts, the approve step finds the row already
//! approved).
//!
//! ## Naming
//!
//! MVP is GitHub-only (issue #100), so the kind ids here are
//! GitHub-projection names:
//!
//! - `Issue` — previously `Spec`; kept as "Spec" alias in the legacy
//!   `github-issue` workflow-builtin for one release cycle.
//! - `PR` — previously `PullRequest`; shipped alongside a `PullRequest`
//!   alias so old catalog consumers keep resolving.

use crate::registry::{MergeRule, RegistryId, TypeDefinition};
use crate::registry_store::RegistryStore;

/// Actor recorded on events produced by this bootstrap function.
pub const CATALOG_ACTOR: &str = "catalog-bootstrap";

/// JSON Schema for the `Issue` intrinsic fields. Issues are thin — title /
/// body / labels / state live on GitHub (source of truth); we keep a
/// reference and let the adapter fetch fresh.
pub fn issue_intrinsic_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "number": { "type": "integer" },
            "title":  { "type": "string"  },
            "state":  { "type": "string",
                        "enum": ["open", "closed"] },
            "labels": { "type": "array",
                        "items": { "type": "string" } }
        },
        "required": ["number"]
    })
}

/// JSON Schema for the `PR` intrinsic fields (issue #103). Gates write
/// partial updates; the `DeepMerge` rule folds them in.
pub fn pr_intrinsic_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "number":  { "type": "integer" },
            "target":  { "type": "object",
                         "properties": {
                             "repo":   { "type": "string" },
                             "branch": { "type": "string" }
                         } },
            "commits": { "type": "array",
                         "items": {
                             "type": "object",
                             "properties": {
                                 "sha":     { "type": "string" },
                                 "message": { "type": "string" },
                                 "author":  { "type": "string" }
                             },
                             "required": ["sha"]
                         } },
            "checks":  { "type": "object",
                         "additionalProperties": {
                             "type": "object",
                             "properties": {
                                 "status":     { "type": "string" },
                                 "conclusion": { "type": "string" }
                             }
                         } },
            "reviews": { "type": "object",
                         "additionalProperties": {
                             "type": "object",
                             "properties": {
                                 "state":       { "type": "string" },
                                 "submitted_at":{ "type": "string",
                                                  "format": "date-time" }
                             },
                             "required": ["state"]
                         } },
            "merged":  { "type": ["object", "null"],
                         "properties": {
                             "sha":       { "type": "string" },
                             "merged_at": { "type": "string",
                                            "format": "date-time" }
                         } },
            "closes_issue": { "type": "string" }
        },
        "required": ["number"]
    })
}

/// Produce the engineering catalog's [`TypeDefinition`]s. Callers can use
/// these directly, or pass them to [`register_engineering_catalog`].
pub fn engineering_types() -> Vec<TypeDefinition> {
    vec![
        TypeDefinition {
            type_id: RegistryId::new("Issue"),
            description: "Engineering issue; external ref is a GitHub issue \
                          labelled 'spec' or similar."
                .into(),
            adapter_id: RegistryId::new("github.issue"),
            gate_ids: vec![RegistryId::new("ReviewApproved")],
            producer_profile_id: Some(RegistryId::new("spec-writer")),
            config: serde_json::json!({
                "external_kind": "github.issue",
                "label": "spec",
            }),
            intrinsic_schema: issue_intrinsic_schema(),
            merge_rule: MergeRule::Overwrite,
        },
        TypeDefinition {
            type_id: RegistryId::new("PR"),
            description: "Pull request implementing an Issue; external ref is a GitHub PR. \
                          Rich kind — carries commits, checks, reviews, merged status \
                          as intrinsic fields (issue #103)."
                .into(),
            adapter_id: RegistryId::new("github.pr"),
            gate_ids: vec![
                RegistryId::new("CiGreen"),
                RegistryId::new("ReviewApproved"),
            ],
            producer_profile_id: Some(RegistryId::new("implementer")),
            config: serde_json::json!({
                "external_kind": "github.pr",
            }),
            intrinsic_schema: pr_intrinsic_schema(),
            merge_rule: MergeRule::DeepMerge,
        },
    ]
}

/// Built-in artifact kinds exposed as workflow kinds. The dashboard's
/// `/api/workflow/kinds` endpoint serves this set as its v1 surface; custom
/// kinds registered via the seed-catalog path stay hidden from the
/// workflow-builder picker until listed here.
pub const BUILTIN_WORKFLOW_KINDS: &[&str] = &["Issue", "PR", "Deployment", "Session"];

/// Whether a given artifact-kind id is one of the built-in workflow kinds.
pub fn is_builtin_workflow_kind(kind: &str) -> bool {
    BUILTIN_WORKFLOW_KINDS.contains(&kind)
}

/// Produce the workflow-built-in [`TypeDefinition`]s. Aligned with #100 kind
/// naming: `Issue`, `PR`, plus v1 seeds `Deployment` and `Session` (see
/// issue #105). Distinct from the engineering catalog so old catalog
/// consumers stay unchanged.
pub fn workflow_builtin_types() -> Vec<TypeDefinition> {
    vec![
        TypeDefinition {
            type_id: RegistryId::new("Issue"),
            description: "A GitHub issue surfaced as a factory artifact by a \
                          workflow trigger (v1 built-in)."
                .into(),
            adapter_id: RegistryId::new("github.issue"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "github.issue",
                "builtin": true,
                "aliases": ["Spec", "github-issue"],
            }),
            intrinsic_schema: issue_intrinsic_schema(),
            merge_rule: MergeRule::Overwrite,
        },
        TypeDefinition {
            type_id: RegistryId::new("PR"),
            description: "A GitHub pull request produced by a workflow stage \
                          (v1 built-in). Gates fold partial updates via \
                          DeepMerge — see issue #103."
                .into(),
            adapter_id: RegistryId::new("github.pr"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "github.pr",
                "builtin": true,
                "aliases": ["PullRequest", "github-pr"],
            }),
            intrinsic_schema: pr_intrinsic_schema(),
            merge_rule: MergeRule::DeepMerge,
        },
        TypeDefinition {
            type_id: RegistryId::new("Deployment"),
            description:
                "A rollout of a merged commit to an environment (v1 built-in, issue #105).".into(),
            adapter_id: RegistryId::new("deployment.local"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "deployment",
                "builtin": true,
            }),
            intrinsic_schema: deployment_intrinsic_schema(),
            merge_rule: MergeRule::DeepMerge,
        },
        TypeDefinition {
            type_id: RegistryId::new("Session"),
            description: "Stiglab agent session promoted to a first-class \
                          registered artifact (v1 built-in, issue #105)."
                .into(),
            adapter_id: RegistryId::new("stiglab.session"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "stiglab.session",
                "builtin": true,
            }),
            intrinsic_schema: session_intrinsic_schema(),
            merge_rule: MergeRule::DeepMerge,
        },
    ]
}

/// JSON Schema for `Deployment` intrinsic fields (issue #105).
pub fn deployment_intrinsic_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "env":          { "type": "string" },
            "sha":          { "type": "string" },
            "status":       { "type": "string",
                              "enum": ["queued", "running", "success",
                                       "failed", "rolled_back"] },
            "url":          { "type": ["string", "null"] },
            "started_at":   { "type": "string", "format": "date-time" },
            "completed_at": { "type": ["string", "null"],
                              "format": "date-time" },
            "logs_ref":     { "type": ["string", "null"] },
            "from_pr":      { "type": ["string", "null"] }
        },
        "required": ["env", "sha", "status"]
    })
}

/// JSON Schema for `Session` intrinsic fields (issue #105). Stiglab's
/// `Session` struct is the source of truth; the schema mirrors its
/// public-surface fields so the registry can project it uniformly.
pub fn session_intrinsic_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "session_id": { "type": "string" },
            "state":      { "type": "string" },
            "works_on":   { "type": ["string", "null"] },
            "produced":   { "type": "array",
                            "items": { "type": "string" } }
        },
        "required": ["session_id", "state"]
    })
}

/// Register the workflow built-in kinds through the registry (propose +
/// approve). Idempotent. Parallels [`register_engineering_catalog`].
pub async fn register_workflow_builtin_kinds(
    store: &RegistryStore,
) -> anyhow::Result<CatalogOutcome> {
    let mut outcome = CatalogOutcome::default();
    for def in workflow_builtin_types() {
        if store.propose_type(&def, CATALOG_ACTOR).await? {
            outcome.proposed += 1;
        }
        if store
            .approve_type(def.type_id.as_str(), CATALOG_ACTOR)
            .await?
        {
            outcome.approved += 1;
        }
    }
    Ok(outcome)
}

/// Summary of what [`register_engineering_catalog`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatalogOutcome {
    pub proposed: usize,
    pub approved: usize,
}

/// Register the minimum engineering catalog through the registry (propose +
/// approve). Idempotent.
pub async fn register_engineering_catalog(store: &RegistryStore) -> anyhow::Result<CatalogOutcome> {
    let mut outcome = CatalogOutcome::default();
    for def in engineering_types() {
        if store.propose_type(&def, CATALOG_ACTOR).await? {
            outcome.proposed += 1;
        }
        if store
            .approve_type(def.type_id.as_str(), CATALOG_ACTOR)
            .await?
        {
            outcome.approved += 1;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engineering_types_are_well_formed() {
        let types = engineering_types();
        let ids: Vec<_> = types.iter().map(|t| t.type_id.as_str()).collect();
        assert!(ids.contains(&"Issue"));
        assert!(ids.contains(&"PR"));
        for t in &types {
            assert!(!t.adapter_id.as_str().is_empty());
            assert!(t.producer_profile_id.is_some());
        }
    }

    #[test]
    fn pr_kind_uses_deep_merge() {
        let pr = engineering_types()
            .into_iter()
            .find(|t| t.type_id.as_str() == "PR")
            .expect("PR must be registered");
        assert_eq!(pr.merge_rule, MergeRule::DeepMerge);
        assert!(pr.intrinsic_schema.get("properties").is_some());
    }

    #[test]
    fn issue_kind_uses_overwrite() {
        let issue = engineering_types()
            .into_iter()
            .find(|t| t.type_id.as_str() == "Issue")
            .expect("Issue must be registered");
        assert_eq!(issue.merge_rule, MergeRule::Overwrite);
    }

    #[test]
    fn workflow_builtin_types_cover_v1_kinds() {
        let types = workflow_builtin_types();
        let ids: Vec<_> = types.iter().map(|t| t.type_id.as_str()).collect();
        assert!(ids.contains(&"Issue"));
        assert!(ids.contains(&"PR"));
        assert!(ids.contains(&"Deployment"));
        assert!(ids.contains(&"Session"));
    }

    #[test]
    fn builtin_predicate_matches_known_kinds() {
        assert!(is_builtin_workflow_kind("Issue"));
        assert!(is_builtin_workflow_kind("PR"));
        assert!(is_builtin_workflow_kind("Deployment"));
        assert!(is_builtin_workflow_kind("Session"));
        assert!(!is_builtin_workflow_kind("custom-kind"));
    }

    #[test]
    fn legacy_alias_preserved_in_config() {
        let issue = workflow_builtin_types()
            .into_iter()
            .find(|t| t.type_id.as_str() == "Issue")
            .unwrap();
        let aliases = issue
            .config
            .get("aliases")
            .and_then(|v| v.as_array())
            .expect("Issue should carry legacy aliases in config");
        let alias_strs: Vec<_> = aliases.iter().filter_map(|v| v.as_str()).collect();
        assert!(alias_strs.contains(&"Spec"));
        assert!(alias_strs.contains(&"github-issue"));
    }
}
