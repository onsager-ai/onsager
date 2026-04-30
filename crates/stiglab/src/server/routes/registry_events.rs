//! Event-type registry manifest read API (spec #131 Lever E / #150).
//!
//! Surfaces `onsager_registry::EVENTS` to the dashboard as JSON. The
//! manifest is the static source of truth for which subsystem produces
//! and consumes each `FactoryEventKind` variant; this route is the
//! runtime read path that lets the dashboard render the catalog without
//! hardcoding the list.
//!
//! Public by design (matches `workflow_kinds`): the manifest is part of
//! the architecture documentation, not user data.

use axum::response::{IntoResponse, Response};
use axum::Json;
use onsager_registry::EVENTS;

/// GET /api/registry/events — return the event-type registry manifest.
pub async fn list_events() -> Response {
    Json(&EVENTS).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn list_returns_manifest_as_json() {
        let resp = list_events().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let events = parsed["events"].as_array().expect("events array");
        assert_eq!(events.len(), EVENTS.events.len());
        // Spot-check shape.
        let first = &events[0];
        assert!(first["kind"].as_str().is_some());
        assert!(first["producers"].as_array().is_some());
        assert!(first["consumers"].as_array().is_some());
        assert!(first["audit_only"].as_bool().is_some());
    }

    #[tokio::test]
    async fn manifest_contains_known_kinds() {
        let resp = list_events().await;
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let kinds: Vec<&str> = parsed["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"forge.shaping_dispatched"));
        assert!(kinds.contains(&"synodic.gate_verdict"));
        assert!(kinds.contains(&"ising.insight_emitted"));
    }
}
