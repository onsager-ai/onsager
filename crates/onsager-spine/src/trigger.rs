//! Canonical workflow trigger type — Lever-E-style foundation for spec
//! #237 (parent #236). This module is the **single source of truth** for
//! what kinds of triggers exist in the factory and the per-kind config
//! they carry.
//!
//! The type doubles as wire format: it serializes to the JSON shape stored
//! in `workflows.trigger_config` (the `kind_tag` lives in
//! `workflows.trigger_kind`, the rest of the variant fields in the JSONB
//! column). Persistence layers reconstruct a [`TriggerKind`] from that
//! `(kind, config)` pair via [`TriggerKind::from_storage`].
//!
//! Adding a new variant is a three-step contract that the registry
//! manifest at `crates/onsager-registry/src/triggers.rs` enforces:
//! 1. Add a variant here with its config fields.
//! 2. Add a row to the registry manifest with the snake-case `kind_tag`.
//! 3. Wire a producer + consumer (or tag the manifest row `audit_only`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A workflow-runtime trigger and its configuration.
///
/// `serde` representation is `tag = "kind"` with snake_case keys, matching
/// the persisted `workflows.trigger_kind` column and `FactoryEventKind`'s
/// wire form. New variants append at the end; do not reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerKind {
    /// A GitHub `issues.labeled` webhook whose label matches `label`.
    /// `repo` is the `"owner/name"` slug.
    GithubIssueWebhook { repo: String, label: String },
}

impl TriggerKind {
    /// Stable snake-case key for the variant; matches the value stored in
    /// `workflows.trigger_kind` and the `kind_tag` column in the trigger
    /// registry manifest.
    pub const fn kind_tag(&self) -> &'static str {
        match self {
            TriggerKind::GithubIssueWebhook { .. } => "github_issue_webhook",
        }
    }
}

/// Errors returned by [`TriggerKind::from_storage`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TriggerStorageError {
    #[error("unknown trigger kind: {0}")]
    UnknownKind(String),
    #[error("trigger config for {kind} failed to parse: {message}")]
    InvalidConfig { kind: &'static str, message: String },
}

impl TriggerKind {
    /// Reconstruct a [`TriggerKind`] from the persisted `(kind_tag, config)`
    /// split. The kind tag selects the variant; the JSONB blob supplies
    /// its config fields. Symmetric to [`TriggerKind::to_storage`].
    pub fn from_storage(
        kind_tag: &str,
        config: &serde_json::Value,
    ) -> Result<Self, TriggerStorageError> {
        match kind_tag {
            "github_issue_webhook" => {
                let repo = config
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TriggerStorageError::InvalidConfig {
                        kind: "github_issue_webhook",
                        message: "missing or non-string `repo`".into(),
                    })?
                    .to_string();
                let label = config
                    .get("label")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TriggerStorageError::InvalidConfig {
                        kind: "github_issue_webhook",
                        message: "missing or non-string `label`".into(),
                    })?
                    .to_string();
                Ok(TriggerKind::GithubIssueWebhook { repo, label })
            }
            other => Err(TriggerStorageError::UnknownKind(other.to_string())),
        }
    }

    /// Split into the persisted `(kind_tag, config)` shape.
    /// `config` is the JSON object stored in `workflows.trigger_config`.
    pub fn to_storage(&self) -> (&'static str, serde_json::Value) {
        match self {
            TriggerKind::GithubIssueWebhook { repo, label } => (
                "github_issue_webhook",
                serde_json::json!({ "repo": repo, "label": label }),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn variant_serializes_with_kind_tag() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "ai".into(),
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "github_issue_webhook");
        assert_eq!(v["repo"], "owner/name");
        assert_eq!(v["label"], "ai");
    }

    #[test]
    fn variant_round_trips_through_serde() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "ai".into(),
        };
        let v = serde_json::to_value(&t).unwrap();
        let back: TriggerKind = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn kind_tag_is_snake_case() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "x".into(),
            label: "y".into(),
        };
        assert_eq!(t.kind_tag(), "github_issue_webhook");
    }

    #[test]
    fn from_storage_reconstructs_variant() {
        let cfg = json!({"repo": "owner/name", "label": "ai"});
        let t = TriggerKind::from_storage("github_issue_webhook", &cfg).unwrap();
        assert_eq!(
            t,
            TriggerKind::GithubIssueWebhook {
                repo: "owner/name".into(),
                label: "ai".into(),
            }
        );
    }

    #[test]
    fn from_storage_rejects_unknown_kind() {
        let err = TriggerKind::from_storage("polling", &json!({})).unwrap_err();
        assert!(matches!(err, TriggerStorageError::UnknownKind(ref s) if s == "polling"));
    }

    #[test]
    fn from_storage_rejects_missing_config_fields() {
        let err =
            TriggerKind::from_storage("github_issue_webhook", &json!({"repo": "a/b"})).unwrap_err();
        assert!(
            matches!(err, TriggerStorageError::InvalidConfig { kind, .. } if kind == "github_issue_webhook")
        );
    }

    #[test]
    fn to_storage_round_trips_through_from_storage() {
        let original = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "planned".into(),
        };
        let (kind, cfg) = original.to_storage();
        let back = TriggerKind::from_storage(kind, &cfg).unwrap();
        assert_eq!(back, original);
    }
}
