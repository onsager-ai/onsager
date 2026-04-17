//! Integration test: registry propose/approve/deprecate flow + engineering
//! catalog bootstrap (issue #14 phase 1).
//!
//! Skipped unless `DATABASE_URL` is set.

use onsager_spine::{
    apply_seed, register_engineering_catalog, EventStore, RegistryKind, RegistryStatus,
    RegistryStore, SeedCatalog, TypeDefinition,
};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

async fn wipe(store: &EventStore, workspace: &str) {
    // All deletes are scoped to the workspace under test so tests on shared
    // or parallel DB backends don't clobber each other.
    let scoped = [
        "DELETE FROM events WHERE stream_type = 'registry' \
         AND data->'event'->>'workspace_id' = $1",
        "DELETE FROM registry_seed_marker WHERE workspace_id = $1",
        "DELETE FROM artifact_types WHERE workspace_id = $1",
        "DELETE FROM artifact_adapters WHERE workspace_id = $1",
        "DELETE FROM gate_evaluators WHERE workspace_id = $1",
        "DELETE FROM agent_profiles WHERE workspace_id = $1",
    ];
    for sql in scoped {
        let _ = sqlx::query(sql).bind(workspace).execute(store.pool()).await;
    }
}

#[tokio::test]
async fn propose_then_approve_type_lifecycle() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let store = EventStore::connect(&url).await.unwrap();
    let workspace = "test_registry_flow";
    wipe(&store, workspace).await;

    let registry = RegistryStore::new(store.clone()).with_workspace(workspace);

    let def = TypeDefinition {
        type_id: "DraftType".into(),
        description: "for test only".into(),
        adapter_id: "registry.local".into(),
        gate_ids: vec!["HumanApproval".into()],
        producer_profile_id: Some("Human".into()),
        config: serde_json::json!({}),
    };

    // propose inserts once
    assert!(registry.propose_type(&def, "alice").await.unwrap());
    // propose is idempotent (second call is a no-op)
    assert!(!registry.propose_type(&def, "alice").await.unwrap());

    let row = registry
        .get(RegistryKind::Type, "DraftType")
        .await
        .unwrap()
        .expect("row present");
    assert_eq!(row.status, RegistryStatus::Proposed);
    assert_eq!(row.revision, 1);

    // approve flips status and emits a second event
    assert!(registry.approve_type("DraftType", "bob").await.unwrap());
    assert!(!registry.approve_type("DraftType", "bob").await.unwrap()); // idempotent

    let row = registry
        .get(RegistryKind::Type, "DraftType")
        .await
        .unwrap()
        .expect("row present");
    assert_eq!(row.status, RegistryStatus::Approved);

    // deprecate emits the terminal event
    assert!(registry
        .deprecate_type("DraftType", "superseded", "bob")
        .await
        .unwrap());
    assert!(!registry
        .deprecate_type("DraftType", "noop", "bob")
        .await
        .unwrap());

    // Three registry events should have landed on the spine for this
    // workspace. Filter on event_type (authoritative column) and
    // workspace_id inside the envelope so we don't pick up other tests'
    // rows on a shared DB.
    let events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events \
         WHERE stream_type = 'registry' \
           AND event_type IN ('registry.type_proposed','registry.type_approved','registry.type_deprecated') \
           AND data->'event'->>'workspace_id' = $1",
    )
    .bind(workspace)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(
        events.0 >= 3,
        "expected at least 3 events, got {}",
        events.0
    );

    wipe(&store, workspace).await;
}

/// The engineering catalog registers both Spec and PullRequest through the
/// registry. Running twice is a no-op.
#[tokio::test]
async fn engineering_catalog_is_idempotent() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let store = EventStore::connect(&url).await.unwrap();
    let workspace = "test_engineering_catalog";
    wipe(&store, workspace).await;

    // The catalog references HumanApproval / Human via base seed, but our
    // propose_type does not enforce referential integrity, so the catalog
    // alone is sufficient for this test.
    let mut seed = SeedCatalog::from_yaml(include_str!("../seeds/base.yaml")).expect("base.yaml");
    seed.workspace_id = Some(workspace.to_owned());
    let _ = apply_seed(&store, &seed).await.unwrap();

    let registry = RegistryStore::new(store.clone()).with_workspace(workspace);

    let first = register_engineering_catalog(&registry).await.unwrap();
    assert_eq!(first.proposed, 2);
    assert_eq!(first.approved, 2);

    let second = register_engineering_catalog(&registry).await.unwrap();
    assert_eq!(second.proposed, 0);
    assert_eq!(second.approved, 0);

    let spec = registry
        .get(RegistryKind::Type, "Spec")
        .await
        .unwrap()
        .expect("Spec registered");
    assert_eq!(spec.status, RegistryStatus::Approved);

    let pr = registry
        .get(RegistryKind::Type, "PullRequest")
        .await
        .unwrap()
        .expect("PullRequest registered");
    assert_eq!(pr.status, RegistryStatus::Approved);

    wipe(&store, workspace).await;
}
