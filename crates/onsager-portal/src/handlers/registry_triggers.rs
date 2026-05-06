//! Trigger-kind registry manifest read API (spec #237 / parent #236).
//!
//! Surfaces `onsager_registry::TRIGGERS` to the dashboard as JSON. The
//! manifest is the static source of truth for what trigger kinds exist
//! and their per-kind metadata; this route is the runtime read path that
//! lets the dashboard's `<TriggerKindPicker>` render the catalog without
//! hardcoding the list.
//!
//! Public by design (matches `registry_events`): the manifest is part of
//! the architecture documentation, not user data.
//!
//! Spec #257 (sub-issue of #222) moved this route from stiglab to
//! portal so the dashboard's `API_BASE` cutover (#222 Slice 6) can
//! eventually drop the `routes::portal::proxy` shim.

use axum::response::{IntoResponse, Response};
use axum::Json;
use onsager_registry::TRIGGERS;

/// GET /api/registry/triggers — return the trigger-kind registry manifest.
pub async fn list_triggers() -> Response {
    Json(&TRIGGERS).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn list_returns_manifest_as_json() {
        let resp = list_triggers().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let triggers = parsed["triggers"].as_array().expect("triggers array");
        assert_eq!(triggers.len(), TRIGGERS.triggers.len());
        // Spot-check shape.
        let first = &triggers[0];
        assert!(first["kind_tag"].as_str().is_some());
        assert!(first["producer"].as_str().is_some());
        assert!(first["category"].as_str().is_some());
        assert!(first["ui_kind"].as_str().is_some());
        assert!(first["description"].as_str().is_some());
    }

    #[tokio::test]
    async fn manifest_contains_github_issue_webhook() {
        let resp = list_triggers().await;
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let kinds: Vec<&str> = parsed["triggers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["kind_tag"].as_str())
            .collect();
        assert!(kinds.contains(&"github_issue_webhook"));
    }
}
