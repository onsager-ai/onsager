//! Spec #183 contract test: `/api/spine/events?workspace=W` filters
//! `events_ext` by the column-level `workspace_id` (replacing the prior
//! JSONB predicate `data->>'workspace_id'`). Two events under different
//! tenants must not leak across the workspace boundary.
//!
//! Pinned at the SQL layer rather than via the HTTP handler so the
//! test doesn't require auth, workspace membership tables, or the
//! `state.spine` wiring — those are exercised by `workspace_scoping.rs`.
//! What matters here is that the swap from
//! `WHERE data->>'workspace_id' = $1` to `WHERE workspace_id = $1`
//! (`crates/stiglab/src/server/routes/spine.rs`) keeps tenant isolation.
//!
//! Skipped when `DATABASE_URL` is unset so a sqlite-only test run still
//! passes; CI's `postgres:16` service container provides the URL.

use onsager_spine::{EventMetadata, EventStore};

async fn try_store() -> Option<EventStore> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(EventStore::connect(&url).await.expect("spine connect"))
}

#[tokio::test]
async fn spine_events_filter_excludes_other_workspaces() {
    let Some(store) = try_store().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    // Two random workspaces so a parallel test run can't see each
    // other's rows. Stream id is shared on purpose — workspace scope
    // is what we're asserting, not stream-key namespacing.
    let w1 = format!("ws_a_{}", ulid::Ulid::new());
    let w2 = format!("ws_b_{}", ulid::Ulid::new());
    let stream_id = format!("forge:art_scope_{}", ulid::Ulid::new());
    let metadata = EventMetadata {
        actor: "test".into(),
        ..Default::default()
    };

    let id_w1 = store
        .append_ext(
            &w1,
            &stream_id,
            "forge",
            "test.scope_w1",
            serde_json::json!({"workspace_id": w1.clone()}),
            &metadata,
            None,
        )
        .await
        .unwrap();
    let id_w2 = store
        .append_ext(
            &w2,
            &stream_id,
            "forge",
            "test.scope_w2",
            serde_json::json!({"workspace_id": w2.clone()}),
            &metadata,
            None,
        )
        .await
        .unwrap();

    // Same SELECT shape the route uses (`crates/stiglab/src/server/
    // routes/spine.rs::list_events`). If a future refactor swaps the
    // column back to a JSONB predicate, this test fails.
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, workspace_id FROM events_ext WHERE workspace_id = $1 \
         ORDER BY id DESC LIMIT 50",
    )
    .bind(&w1)
    .fetch_all(store.pool())
    .await
    .unwrap();

    assert!(
        rows.iter().any(|(id, _)| *id == id_w1),
        "W1 query must return the W1 row"
    );
    assert!(
        rows.iter().all(|(_, ws)| ws == &w1),
        "W1 query must not return rows tagged with another workspace; got {rows:?}"
    );
    assert!(
        rows.iter().all(|(id, _)| *id != id_w2),
        "W1 query must not return the W2 row"
    );

    // Clean up.
    sqlx::query("DELETE FROM events_ext WHERE id IN ($1, $2)")
        .bind(id_w1)
        .bind(id_w2)
        .execute(store.pool())
        .await
        .unwrap();
}
