//! Integration tests for the Activity-tab operator-grain filter
//! (ADR 0019 / spec #465).
//!
//! Verifies two things against a real Postgres:
//! 1. `operator_grain=true` returns *exactly* the rows whose
//!    `event_type` is in `onsager_registry::EVENTS`'s operator-grain
//!    allowlist — no internal subsystem dispatches or analyzer
//!    diagnostics leak through.
//! 2. The path-scoped alias `/api/workspaces/:id/activity` behaves
//!    identically to `/api/spine/events?workspace=W&operator_grain=true`
//!    — same allowlist, same projection. Verified by driving the same
//!    `query_events_for_workspace` helper both code paths share.
//!
//! Skipped when `DATABASE_URL` is unset — the filter lives in SQL, so
//! the contract only holds against Postgres.

use chrono::Utc;
use onsager_portal::handlers::spine::{EventsQuery, query_events_for_workspace};
use onsager_registry::EVENTS;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

async fn cleanup_workspace(pool: &PgPool, workspace_id: &str) {
    let _ = sqlx::query("DELETE FROM events_ext WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(pool)
        .await;
}

/// Insert a single events_ext row.
async fn insert_event(pool: &PgPool, workspace_id: &str, event_type: &str, namespace: &str) {
    sqlx::query(
        "INSERT INTO events_ext (workspace_id, stream_id, namespace, event_type, data, metadata, created_at) \
         VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7)",
    )
    .bind(workspace_id)
    .bind(format!("stream-{}", Uuid::new_v4()))
    .bind(namespace)
    .bind(event_type)
    .bind(json!({ "test": true }))
    .bind(json!({ "actor": "test" }))
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("insert events_ext row");
}

/// Seed one row per representative event_type covering both sides of
/// the operator-grain partition. Returns the workspace_id used.
async fn seed_mixed_events(pool: &PgPool) -> String {
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    // Operator-grain ✓
    insert_event(pool, &workspace_id, "session.completed", "portal").await;
    insert_event(pool, &workspace_id, "verify.verdict", "substrate").await;
    insert_event(pool, &workspace_id, "trigger.fired", "substrate").await;
    insert_event(pool, &workspace_id, "stage.entered", "substrate").await;
    insert_event(pool, &workspace_id, "node.failed", "substrate").await;
    insert_event(pool, &workspace_id, "artifact.state_changed", "substrate").await;
    // Not operator-grain ✗
    insert_event(pool, &workspace_id, "forge.insight_observed", "substrate").await;
    insert_event(pool, &workspace_id, "forge.decision_made", "substrate").await;
    workspace_id
}

fn expected_operator_grain_kinds() -> std::collections::HashSet<&'static str> {
    EVENTS
        .events
        .iter()
        .filter(|e| e.operator_grain)
        .map(|e| e.kind)
        .collect()
}

/// With `operator_grain=true`, the SQL filter must include *only*
/// kinds the registry marks operator_grain. None of the seeded
/// internal-dispatch rows should appear.
#[tokio::test]
async fn operator_grain_true_filters_to_registry_allowlist() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = seed_mixed_events(&pool).await;

    let params = EventsQuery {
        workspace: workspace_id.clone(),
        stream_type: None,
        event_type: None,
        stream_id: None,
        run_id: None,
        operator_grain: Some(true),
        limit: Some(500),
    };
    let events = query_events_for_workspace(&pool, &workspace_id, &params)
        .await
        .expect("query");
    let allowlist = expected_operator_grain_kinds();
    assert!(
        !events.is_empty(),
        "expected at least one operator-grain row to come back"
    );
    for e in &events {
        assert!(
            allowlist.contains(e.event_type.as_str()),
            "leaked non-operator-grain event: {}",
            e.event_type
        );
    }
    // The six allowlisted kinds we seeded should each appear at least once.
    let got: std::collections::HashSet<&str> =
        events.iter().map(|e| e.event_type.as_str()).collect();
    for kind in [
        "session.completed",
        "verify.verdict",
        "trigger.fired",
        "stage.entered",
        "node.failed",
        "artifact.state_changed",
    ] {
        assert!(
            got.contains(kind),
            "missing seeded operator-grain kind: {kind}"
        );
    }
    // And the five non-operator-grain rows should be absent.
    for kind in ["forge.insight_observed", "forge.decision_made"] {
        assert!(
            !got.contains(kind),
            "non-operator-grain kind leaked into the activity feed: {kind}"
        );
    }

    cleanup_workspace(&pool, &workspace_id).await;
}

/// Backward compatibility: with `operator_grain=None` (the default for
/// `/api/spine/events`), every seeded row is returned — the filter is
/// strictly opt-in.
#[tokio::test]
async fn operator_grain_unset_returns_full_stream() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = seed_mixed_events(&pool).await;

    let params = EventsQuery {
        workspace: workspace_id.clone(),
        stream_type: None,
        event_type: None,
        stream_id: None,
        run_id: None,
        operator_grain: None,
        limit: Some(500),
    };
    let events = query_events_for_workspace(&pool, &workspace_id, &params)
        .await
        .expect("query");
    let got: std::collections::HashSet<&str> =
        events.iter().map(|e| e.event_type.as_str()).collect();
    // Both sides of the partition are present.
    assert!(got.contains("session.completed"));
    assert!(got.contains("forge.insight_observed"));
    assert!(got.contains("forge.decision_made"));

    cleanup_workspace(&pool, &workspace_id).await;
}

/// `operator_grain=true` stacks with `event_type` — the allowlist
/// narrows to that one kind, not the full feed.
#[tokio::test]
async fn operator_grain_and_event_type_compose() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = seed_mixed_events(&pool).await;

    let params = EventsQuery {
        workspace: workspace_id.clone(),
        stream_type: None,
        event_type: Some("verify.verdict".into()),
        stream_id: None,
        run_id: None,
        operator_grain: Some(true),
        limit: Some(500),
    };
    let events = query_events_for_workspace(&pool, &workspace_id, &params)
        .await
        .expect("query");
    assert!(
        !events.is_empty(),
        "expected at least one verify.verdict row"
    );
    for e in &events {
        assert_eq!(e.event_type, "verify.verdict");
    }

    cleanup_workspace(&pool, &workspace_id).await;
}

/// The path-scoped alias defaults `operator_grain` to `true`. Since the
/// alias just constructs an `EventsQuery` and delegates, verify the
/// post-construction `EventsQuery` produces the same filtered output
/// regardless of which entry-point we drive.
#[tokio::test]
async fn path_scoped_alias_matches_operator_grain_query() {
    let Some(pool) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = seed_mixed_events(&pool).await;

    // What the path-scoped handler builds:
    let alias_params = EventsQuery {
        workspace: workspace_id.clone(),
        stream_type: None,
        event_type: None,
        stream_id: None,
        run_id: None,
        operator_grain: Some(true),
        limit: None,
    };
    // What an MCP client would build:
    let mcp_params = EventsQuery {
        workspace: workspace_id.clone(),
        stream_type: None,
        event_type: None,
        stream_id: None,
        run_id: None,
        operator_grain: Some(true),
        limit: None,
    };

    let a = query_events_for_workspace(&pool, &workspace_id, &alias_params)
        .await
        .expect("alias query");
    let b = query_events_for_workspace(&pool, &workspace_id, &mcp_params)
        .await
        .expect("query-param query");
    let a_kinds: Vec<&str> = a.iter().map(|e| e.event_type.as_str()).collect();
    let b_kinds: Vec<&str> = b.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(a_kinds, b_kinds, "alias and query-param surfaces diverged");

    cleanup_workspace(&pool, &workspace_id).await;
}
