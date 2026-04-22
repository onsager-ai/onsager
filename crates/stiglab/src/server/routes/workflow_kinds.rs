//! Workflow artifact-kind listing (issue #102).
//!
//! Serves the registry's built-in workflow kinds to the dashboard so the
//! workflow builder can render the kind picker from runtime data instead of
//! a hardcoded TypeScript union. V1 returns only the registry's builtin
//! catalog; custom kinds registered later via the registry seed path stay
//! hidden from mobile/chat workflow selectors until explicitly surfaced
//! here (matches the `BUILTIN_WORKFLOW_KINDS` filter in
//! `onsager-registry::catalog`).

use axum::response::{IntoResponse, Response};
use axum::Json;
use onsager_registry::catalog::{workflow_builtin_types, BUILTIN_WORKFLOW_KINDS};
use onsager_registry::registry::MergeRule;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct WorkflowKind {
    pub id: String,
    pub description: String,
    pub merge_rule: MergeRule,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_kind: Option<String>,
    pub aliases: Vec<String>,
    pub intrinsic_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub kinds: Vec<WorkflowKind>,
}

/// GET /api/workflow/kinds — return the registry's built-in workflow kinds.
///
/// Public by design: the dashboard fetches this without a session to render
/// the workflow-builder's kind picker before the user has picked a tenant.
pub async fn list_workflow_kinds() -> Response {
    let kinds = workflow_builtin_types()
        .into_iter()
        .filter(|def| BUILTIN_WORKFLOW_KINDS.contains(&def.type_id.as_str()))
        .map(|def| {
            let external_kind = def
                .config
                .get("external_kind")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let aliases = def
                .config
                .get("aliases")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            WorkflowKind {
                id: def.type_id.as_str().to_owned(),
                description: def.description,
                merge_rule: def.merge_rule,
                external_kind,
                aliases,
                intrinsic_schema: def.intrinsic_schema,
            }
        })
        .collect();

    Json(ListResponse { kinds }).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn list_returns_builtin_kinds_as_json() {
        let resp = list_workflow_kinds().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids: Vec<_> = parsed["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains(&"Issue".to_owned()));
        assert!(ids.contains(&"PR".to_owned()));
        assert!(ids.contains(&"Deployment".to_owned()));
        assert!(ids.contains(&"Session".to_owned()));
    }

    #[tokio::test]
    async fn issue_kind_carries_spec_alias() {
        let resp = list_workflow_kinds().await;
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let issue = parsed["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["id"].as_str() == Some("Issue"))
            .expect("Issue kind in response");
        let aliases: Vec<_> = issue["aliases"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a.as_str())
            .collect();
        assert!(aliases.contains(&"Spec"));
    }
}
