//! First registered catalog — the engineering pipeline (Spec → PullRequest).
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

use crate::registry::{RegistryId, TypeDefinition};
use crate::registry_store::RegistryStore;

/// Actor recorded on events produced by this bootstrap function.
pub const CATALOG_ACTOR: &str = "catalog-bootstrap";

/// Produce the engineering catalog's [`TypeDefinition`]s. Callers can use
/// these directly, or pass them to [`register_engineering_catalog`].
pub fn engineering_types() -> Vec<TypeDefinition> {
    vec![
        TypeDefinition {
            type_id: RegistryId::new("Spec"),
            description: "Engineering specification; external ref is a GitHub issue \
                          labelled 'spec'."
                .into(),
            adapter_id: RegistryId::new("github.issue"),
            gate_ids: vec![RegistryId::new("ReviewApproved")],
            producer_profile_id: Some(RegistryId::new("spec-writer")),
            config: serde_json::json!({
                "external_kind": "github.issue",
                "label": "spec",
            }),
        },
        TypeDefinition {
            type_id: RegistryId::new("PullRequest"),
            description: "Pull request implementing a Spec; external ref is a GitHub PR.".into(),
            adapter_id: RegistryId::new("github.pr"),
            gate_ids: vec![
                RegistryId::new("CiGreen"),
                RegistryId::new("ReviewApproved"),
            ],
            producer_profile_id: Some(RegistryId::new("implementer")),
            config: serde_json::json!({
                "external_kind": "github.pr",
            }),
        },
    ]
}

/// Built-in artifact kinds exposed to mobile/chat workflow selectors
/// (issue #81). These are the only kinds the dashboard surfaces when a user
/// configures a workflow; custom kinds registered via the seed-catalog path
/// stay hidden from mobile selectors.
pub const BUILTIN_WORKFLOW_KINDS: &[&str] = &["github-issue", "github-pr"];

/// Whether a given artifact-kind id is one of the selectors-API built-ins.
/// Callers behind a "selectors" endpoint filter by this to hide custom
/// types the user hasn't explicitly surfaced.
pub fn is_builtin_workflow_kind(kind: &str) -> bool {
    BUILTIN_WORKFLOW_KINDS.contains(&kind)
}

/// Produce the workflow-built-in [`TypeDefinition`]s — the `github-issue` and
/// `github-pr` kinds referenced from v1 presets. Distinct from the
/// engineering catalog so old catalog consumers stay unchanged.
pub fn workflow_builtin_types() -> Vec<TypeDefinition> {
    vec![
        TypeDefinition {
            type_id: RegistryId::new("github-issue"),
            description: "A GitHub issue surfaced as a factory artifact by a \
                          workflow trigger (v1 built-in)."
                .into(),
            adapter_id: RegistryId::new("github.issue"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "github.issue",
                "builtin": true,
            }),
        },
        TypeDefinition {
            type_id: RegistryId::new("github-pr"),
            description: "A GitHub pull request produced by a workflow \
                          stage (v1 built-in)."
                .into(),
            adapter_id: RegistryId::new("github.pr"),
            gate_ids: vec![],
            producer_profile_id: None,
            config: serde_json::json!({
                "external_kind": "github.pr",
                "builtin": true,
            }),
        },
    ]
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
        assert!(ids.contains(&"Spec"));
        assert!(ids.contains(&"PullRequest"));
        for t in &types {
            assert!(!t.adapter_id.as_str().is_empty());
            assert!(t.producer_profile_id.is_some());
        }
    }

    #[test]
    fn workflow_builtin_types_include_github_issue_and_pr() {
        let types = workflow_builtin_types();
        let ids: Vec<_> = types.iter().map(|t| t.type_id.as_str()).collect();
        assert!(ids.contains(&"github-issue"));
        assert!(ids.contains(&"github-pr"));
    }

    #[test]
    fn builtin_predicate_matches_known_kinds() {
        assert!(is_builtin_workflow_kind("github-issue"));
        assert!(is_builtin_workflow_kind("github-pr"));
        assert!(!is_builtin_workflow_kind("custom-kind"));
        assert!(!is_builtin_workflow_kind("Spec"));
    }
}
