//! Built-in [`GateEvaluator`] implementations.
//!
//! These are pure: they derive the verdict from the [`GateContext::payload`]
//! alone, so they are deterministic, testable, and independent of any external
//! service. Integrations (GitHub CI, reviewer lookups, …) happen further up the
//! stack — they populate the payload; the evaluator decides.
//!
//! All built-ins read their payload keys defensively: if the key is missing
//! the evaluator returns [`GateVerdict::Escalate`] so a human sees the gap,
//! rather than silently allowing or denying.

use async_trait::async_trait;

use crate::registry::{AdapterResult, GateContext, GateEvaluator, GateVerdict, RegistryId};

/// Require explicit human approval before a transition.
///
/// Payload shape: `{ "approved_by": "<actor>", "approved_at": "<rfc3339>" }`.
/// An unset `approved_by` escalates; `approved_by == "deny"` is treated as a
/// deny so reviewers can reject without opening a separate code path.
pub struct HumanApproval {
    id: RegistryId,
}

impl Default for HumanApproval {
    fn default() -> Self {
        Self {
            id: RegistryId::new("HumanApproval"),
        }
    }
}

#[async_trait]
impl GateEvaluator for HumanApproval {
    fn evaluator_id(&self) -> &RegistryId {
        &self.id
    }

    async fn evaluate(&self, ctx: &GateContext) -> AdapterResult<GateVerdict> {
        let approved_by = ctx.payload.get("approved_by").and_then(|v| v.as_str());
        Ok(match approved_by {
            None | Some("") => GateVerdict::Escalate {
                reason: "awaiting human approval".into(),
            },
            Some("deny") => GateVerdict::Deny {
                reason: "human reviewer rejected".into(),
            },
            Some(who) => GateVerdict::Allow {
                reason: format!("approved by {who}"),
            },
        })
    }
}

/// Allow only if CI is green for the external ref.
///
/// Payload shape: `{ "ci_status": "success" | "failure" | "pending" | ... }`.
pub struct CiGreen {
    id: RegistryId,
}

impl Default for CiGreen {
    fn default() -> Self {
        Self {
            id: RegistryId::new("CiGreen"),
        }
    }
}

#[async_trait]
impl GateEvaluator for CiGreen {
    fn evaluator_id(&self) -> &RegistryId {
        &self.id
    }

    async fn evaluate(&self, ctx: &GateContext) -> AdapterResult<GateVerdict> {
        let status = ctx
            .payload
            .get("ci_status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        Ok(match status {
            "success" => GateVerdict::Allow {
                reason: "ci green".into(),
            },
            "failure" | "error" | "cancelled" => GateVerdict::Deny {
                reason: format!("ci {status}"),
            },
            "" => GateVerdict::Escalate {
                reason: "ci status unknown".into(),
            },
            other => GateVerdict::Escalate {
                reason: format!("ci pending ({other})"),
            },
        })
    }
}

/// Allow only if the PR has a non-empty approving review set.
///
/// Payload shape: `{ "approving_reviews": [ "<reviewer>", ... ],
///                   "changes_requested": [ "<reviewer>", ... ] }`.
pub struct ReviewApproved {
    id: RegistryId,
}

impl Default for ReviewApproved {
    fn default() -> Self {
        Self {
            id: RegistryId::new("ReviewApproved"),
        }
    }
}

#[async_trait]
impl GateEvaluator for ReviewApproved {
    fn evaluator_id(&self) -> &RegistryId {
        &self.id
    }

    async fn evaluate(&self, ctx: &GateContext) -> AdapterResult<GateVerdict> {
        let empty_vec = Vec::new();
        let approving = ctx
            .payload
            .get("approving_reviews")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);
        let changes = ctx
            .payload
            .get("changes_requested")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);

        Ok(if !changes.is_empty() {
            GateVerdict::Deny {
                reason: format!("{} reviewer(s) requested changes", changes.len()),
            }
        } else if !approving.is_empty() {
            GateVerdict::Allow {
                reason: format!("{} approval(s)", approving.len()),
            }
        } else {
            GateVerdict::Escalate {
                reason: "no reviews yet".into(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DEFAULT_WORKSPACE;
    use onsager_artifact::ArtifactId;

    fn ctx(payload: serde_json::Value) -> GateContext {
        GateContext {
            artifact_id: ArtifactId::new("art_test"),
            type_id: "Spec".into(),
            workspace_id: DEFAULT_WORKSPACE.into(),
            payload,
        }
    }

    #[tokio::test]
    async fn human_approval_escalates_when_absent() {
        let g = HumanApproval::default();
        let v = g.evaluate(&ctx(serde_json::json!({}))).await.unwrap();
        assert!(matches!(v, GateVerdict::Escalate { .. }));
    }

    #[tokio::test]
    async fn human_approval_allows_when_approved() {
        let g = HumanApproval::default();
        let v = g
            .evaluate(&ctx(serde_json::json!({"approved_by": "marvin"})))
            .await
            .unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn human_approval_denies_on_deny_sentinel() {
        let g = HumanApproval::default();
        let v = g
            .evaluate(&ctx(serde_json::json!({"approved_by": "deny"})))
            .await
            .unwrap();
        assert!(matches!(v, GateVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn ci_green_paths() {
        let g = CiGreen::default();
        assert!(g
            .evaluate(&ctx(serde_json::json!({"ci_status": "success"})))
            .await
            .unwrap()
            .is_allow());
        assert!(matches!(
            g.evaluate(&ctx(serde_json::json!({"ci_status": "failure"})))
                .await
                .unwrap(),
            GateVerdict::Deny { .. }
        ));
        assert!(matches!(
            g.evaluate(&ctx(serde_json::json!({"ci_status": "pending"})))
                .await
                .unwrap(),
            GateVerdict::Escalate { .. }
        ));
    }

    #[tokio::test]
    async fn review_approved_requires_approvals_without_blockers() {
        let g = ReviewApproved::default();
        let v = g
            .evaluate(&ctx(serde_json::json!({"approving_reviews": ["a", "b"]})))
            .await
            .unwrap();
        assert!(v.is_allow());

        let v = g
            .evaluate(&ctx(serde_json::json!({
                "approving_reviews": ["a"],
                "changes_requested": ["b"]
            })))
            .await
            .unwrap();
        assert!(matches!(v, GateVerdict::Deny { .. }));

        let v = g.evaluate(&ctx(serde_json::json!({}))).await.unwrap();
        assert!(matches!(v, GateVerdict::Escalate { .. }));
    }
}
