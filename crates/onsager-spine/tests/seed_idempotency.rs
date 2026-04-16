//! Integration test for the idempotent seed loader.
//!
//! Skipped unless `DATABASE_URL` is set (matches the convention in
//! `store::tests`). Run via `just test-spine`.

use onsager_spine::{apply_seed, EventStore, SeedCatalog, DEFAULT_WORKSPACE};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

async fn cleanup(store: &EventStore, seed_name: &str, workspace_id: &str) {
    // Remove any registry events emitted by previous runs of this test.
    sqlx::query("DELETE FROM events WHERE stream_type = 'registry'")
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM registry_seed_marker WHERE seed_name = $1 AND workspace_id = $2")
        .bind(seed_name)
        .bind(workspace_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM artifact_types WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM artifact_adapters WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM gate_evaluators WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM agent_profiles WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(store.pool())
        .await
        .ok();
}

#[tokio::test]
async fn seed_applies_once_then_is_noop() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let store = EventStore::connect(&url).await.unwrap();

    let mut seed =
        SeedCatalog::from_yaml(include_str!("../seeds/base.yaml")).expect("base.yaml parses");
    // Use an isolated workspace so this test doesn't contend with others.
    let workspace = "test_seed_idempotency";
    seed.workspace_id = Some(workspace.to_owned());
    cleanup(&store, &seed.name, workspace).await;

    // First apply: writes rows + events.
    let first = apply_seed(&store, &seed).await.unwrap();
    assert!(first.applied, "first apply should write");
    assert!(first.events_emitted > 0, "first apply should emit events");

    // Marker is present; rows are populated.
    let types: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM artifact_types WHERE workspace_id = $1")
            .bind(workspace)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(types.0 as usize, seed.types.len());

    // Second apply: marker short-circuits, zero events.
    let second = apply_seed(&store, &seed).await.unwrap();
    assert!(!second.applied, "second apply should be a no-op");
    assert_eq!(second.events_emitted, 0);

    // Events table should contain one registry event per seed entry from the
    // first apply and nothing new from the second.
    let expected_events =
        seed.types.len() + seed.adapters.len() + seed.evaluators.len() + seed.profiles.len();
    let evts: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE stream_type = 'registry' AND metadata->>'actor' = 'seed'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(evts.0 as usize, expected_events);

    cleanup(&store, &seed.name, workspace).await;
}

#[tokio::test]
async fn seed_respects_default_workspace() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let store = EventStore::connect(&url).await.unwrap();

    let mut seed =
        SeedCatalog::from_yaml(include_str!("../seeds/base.yaml")).expect("base.yaml parses");
    // Override the seed name to isolate this test from the base namespace.
    seed.name = "base_default_ws_test".into();
    seed.workspace_id = None;
    cleanup(&store, &seed.name, DEFAULT_WORKSPACE).await;

    let out = apply_seed(&store, &seed).await.unwrap();
    assert!(out.applied);

    let marker: Option<(String,)> =
        sqlx::query_as("SELECT workspace_id FROM registry_seed_marker WHERE seed_name = $1")
            .bind(&seed.name)
            .fetch_optional(store.pool())
            .await
            .unwrap();
    assert_eq!(marker.map(|r| r.0).as_deref(), Some(DEFAULT_WORKSPACE));

    cleanup(&store, &seed.name, DEFAULT_WORKSPACE).await;
}
