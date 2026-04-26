//! Restart-consistency tests for forge state persistence (issue #30).
//!
//! Exercises the two halves of the projection in `forge::core::persistence`:
//!
//! 1. `insert_artifact_row` writes the `artifacts` row in a transaction.
//! 2. `persist_artifact_state` mirrors the in-memory store back to the row.
//! 3. `load_artifact_store` rebuilds the store from the row on "restart".
//!
//! These tests require Postgres with migrations 001–005 applied. They skip
//! themselves when `DATABASE_URL` is unset, matching the convention in
//! `crates/onsager-warehouse/tests/warehouse_flow.rs`.

use forge::core::persistence;
use onsager_artifact::{Artifact, ArtifactId, ArtifactState, ArtifactVersionId, Kind};
use sqlx::PgPool;

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

async fn reset(pool: &PgPool, artifact_id: &str) {
    // Order respects FKs: lineage → versions → warehouse rows → artifacts.
    sqlx::query("DELETE FROM vertical_lineage WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM horizontal_lineage WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM quality_signals WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM artifact_versions WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM deliveries WHERE bundle_id IN (SELECT bundle_id FROM bundles WHERE artifact_id = $1)")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("UPDATE artifacts SET current_version_id = NULL WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM bundles WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM artifacts WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn restart_in_mid_tick_preserves_state_and_bundle() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();

    let artifact_id = "art_test_restart_release_01";
    reset(&pool, artifact_id).await;

    // 1. Register via the DB-first path.
    persistence::insert_artifact_row(&pool, artifact_id, "code", "restart-test", "marvin", None)
        .await
        .expect("insert_artifact_row");

    // 2. Simulate a tick that drove the artifact all the way to Released
    //    and sealed a bundle. The tick only mutates the in-memory store; we
    //    then call `persist_artifact_state` the way serve.rs does after the
    //    lock is released.
    let mut advanced = Artifact::new(Kind::Code, "restart-test", "marvin", "forge", vec![]);
    advanced.artifact_id = ArtifactId::new(artifact_id);
    advanced.state = ArtifactState::Released;
    advanced.current_version = 2;
    advanced.current_version_id = Some(ArtifactVersionId::new("ver_restart_test_abc"));

    persistence::persist_artifact_state(&pool, &advanced)
        .await
        .expect("persist_artifact_state");

    // 3. "Restart": throw away all in-memory state, load fresh from the DB.
    let reloaded = persistence::load_artifact_store(&pool)
        .await
        .expect("load_artifact_store");
    let aid = ArtifactId::new(artifact_id);
    let reloaded_artifact = reloaded
        .get(&aid)
        .expect("reloaded store should contain the artifact");

    assert_eq!(reloaded_artifact.state, ArtifactState::Released);
    assert_eq!(reloaded_artifact.current_version, 2);
    assert_eq!(
        reloaded_artifact
            .current_version_id
            .as_ref()
            .map(|b| b.as_str()),
        Some("ver_restart_test_abc"),
    );

    reset(&pool, artifact_id).await;
}

#[tokio::test]
async fn load_skips_archived_artifacts() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();

    let active_id = "art_test_restart_active_02";
    let archived_id = "art_test_restart_archived_02";
    reset(&pool, active_id).await;
    reset(&pool, archived_id).await;

    persistence::insert_artifact_row(&pool, active_id, "code", "active", "marvin", None)
        .await
        .unwrap();
    persistence::insert_artifact_row(&pool, archived_id, "code", "archived", "marvin", None)
        .await
        .unwrap();

    // Drive the second to Archived state in the DB.
    let mut archived = Artifact::new(Kind::Code, "archived", "marvin", "forge", vec![]);
    archived.artifact_id = ArtifactId::new(archived_id);
    archived.state = ArtifactState::Archived;
    persistence::persist_artifact_state(&pool, &archived)
        .await
        .unwrap();

    let store = persistence::load_artifact_store(&pool).await.unwrap();
    assert!(store.get(&ArtifactId::new(active_id)).is_some());
    assert!(store.get(&ArtifactId::new(archived_id)).is_none());

    reset(&pool, active_id).await;
    reset(&pool, archived_id).await;
}

#[tokio::test]
async fn failed_insert_leaves_no_divergent_state() {
    // Acceptance for #30: "simulated failure between the two writes in
    // register_artifact leaves no divergent state observable to subsequent
    // requests." We close a pool and try to insert through it; the call
    // must fail and no row must appear via `load_artifact_store`. This
    // matches serve.rs's DB-first contract: if `insert_artifact_row`
    // returns Err, the in-memory insert is skipped, so a later request
    // cannot observe a ghost artifact.
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();

    let artifact_id = "art_test_restart_failure_03";
    reset(&pool, artifact_id).await;

    let bad_pool = PgPool::connect(&url).await.unwrap();
    bad_pool.close().await;
    let err = persistence::insert_artifact_row(
        &bad_pool,
        artifact_id,
        "code",
        "failure-test",
        "marvin",
        None,
    )
    .await;
    assert!(err.is_err(), "expected closed-pool insert to fail");

    let store = persistence::load_artifact_store(&pool).await.unwrap();
    assert!(store.get(&ArtifactId::new(artifact_id)).is_none());

    reset(&pool, artifact_id).await;
}
