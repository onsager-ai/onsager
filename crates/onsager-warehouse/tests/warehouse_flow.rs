//! Integration test for the filesystem warehouse backend.
//!
//! Exercises the full seal path against Postgres:
//!   1. Insert a test artifact row.
//!   2. Seal two bundles in sequence.
//!   3. Verify version monotonicity, supersession chain, and fetch idempotency.
//!
//! Skipped unless `DATABASE_URL` is set (matches the convention in
//! `store::tests` and `seed_idempotency`).

use onsager_artifact::ArtifactId;
use onsager_spine::EventStore;
use onsager_warehouse::{FilesystemWarehouse, Outputs, SealRequest, Warehouse};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

async fn reset_artifact(store: &EventStore, artifact_id: &str) {
    // Clean order: deliveries -> bundles -> consumer_sinks -> artifact, since
    // deliveries and bundles FK into the artifacts row.
    sqlx::query("DELETE FROM deliveries WHERE bundle_id IN (SELECT bundle_id FROM bundles WHERE artifact_id = $1)")
        .bind(artifact_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("UPDATE artifacts SET current_bundle_id = NULL WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM bundles WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM consumer_sinks WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(store.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM artifacts WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(store.pool())
        .await
        .ok();
}

#[tokio::test]
async fn seal_chain_produces_monotonic_versions() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let store = EventStore::connect(&url).await.unwrap();

    let artifact_id = ArtifactId::new("art_test_warehouse_01");
    reset_artifact(&store, artifact_id.as_str()).await;

    sqlx::query(
        "INSERT INTO artifacts (artifact_id, kind, name, owner, created_by, state) \
         VALUES ($1, 'code', 'warehouse-test', 'tester', 'tester', 'under_review')",
    )
    .bind(artifact_id.as_str())
    .execute(store.pool())
    .await
    .unwrap();

    let tmpdir = tempfile::tempdir().unwrap();
    let warehouse = FilesystemWarehouse::new(store.pool().clone(), tmpdir.path());

    // First seal: version 1, no predecessor.
    let mut outputs_v1 = Outputs::new();
    outputs_v1.push("README.md", b"hello world".to_vec());
    outputs_v1.push("src/main.rs", b"fn main() {}".to_vec());

    let bundle_v1 = warehouse
        .seal(SealRequest {
            artifact_id: artifact_id.clone(),
            sealed_by: "sess_01".into(),
            metadata: serde_json::json!({"kind": "code"}),
            outputs: outputs_v1,
        })
        .await
        .expect("seal v1");

    assert_eq!(bundle_v1.version, 1);
    assert!(bundle_v1.supersedes.is_none());
    assert_eq!(bundle_v1.manifest.entries.len(), 2);
    // Entries must be sorted by path for canonical manifest hashing.
    assert_eq!(bundle_v1.manifest.entries[0].path, "README.md");
    assert_eq!(bundle_v1.manifest.entries[1].path, "src/main.rs");

    // Second seal: version 2, supersedes v1.
    let mut outputs_v2 = Outputs::new();
    outputs_v2.push("README.md", b"hello universe".to_vec());

    let bundle_v2 = warehouse
        .seal(SealRequest {
            artifact_id: artifact_id.clone(),
            sealed_by: "sess_02".into(),
            metadata: serde_json::json!({"kind": "code", "rework": true}),
            outputs: outputs_v2,
        })
        .await
        .expect("seal v2");

    assert_eq!(bundle_v2.version, 2);
    assert_eq!(bundle_v2.supersedes.as_ref(), Some(&bundle_v1.bundle_id));

    // Fetch round-trip.
    let fetched = warehouse.fetch(&bundle_v2.bundle_id).await.unwrap();
    assert_eq!(fetched, bundle_v2);
    assert!(warehouse.exists(&bundle_v1.bundle_id).await.unwrap());

    reset_artifact(&store, artifact_id.as_str()).await;
}
