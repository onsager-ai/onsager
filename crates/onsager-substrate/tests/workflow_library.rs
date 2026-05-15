//! Integration tests for `WorkflowLibrary` (SUB-04, #351).
//!
//! These tests need a Postgres instance with the spine migrations
//! applied — they skip themselves when `DATABASE_URL` is unset, the
//! same pattern as `forge/tests/persistence_restart.rs`.
//!
//! Each test scopes its writes to a unique `spec_kind` so concurrent
//! runs of the suite don't trip each other on the `(spec_kind,
//! version)` unique constraint.

use onsager_artifact::{ArtifactId, NodeId};
use onsager_substrate::{
    EdgeId, EdgeRef, NoOpExecutor, Workflow, WorkflowLibrary, WorkflowLibraryError,
};
use sqlx::PgPool;

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

fn unique_kind(prefix: &str) -> String {
    format!("{}_{}", prefix, uuid::Uuid::new_v4().simple())
}

fn sample_workflow(label: &str) -> Workflow {
    let edge_in = EdgeId::generate();
    let edge_out = EdgeId::generate();
    Workflow {
        nodes: vec![onsager_substrate::Node {
            id: NodeId::generate(),
            executor: Box::new(NoOpExecutor),
            inputs: vec![EdgeRef::new(edge_in)],
            outputs: vec![EdgeRef::new(edge_out)],
        }],
        edges: vec![
            onsager_substrate::Edge {
                id: edge_in,
                artifact_id: ArtifactId::new(format!("art_in_{label}")),
                requires_deterministic: true,
            },
            onsager_substrate::Edge {
                id: edge_out,
                artifact_id: ArtifactId::new(format!("art_out_{label}")),
                requires_deterministic: false,
            },
        ],
    }
}

async fn cleanup(pool: &PgPool, kind: &str) {
    sqlx::query("DELETE FROM workflow_library WHERE spec_kind = $1")
        .bind(kind)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn register_then_lookup_returns_latest_version() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_register_lookup");
    cleanup(&pool, &kind).await;

    let lib = WorkflowLibrary::new(pool.clone());

    // Register v1 — lookup returns it.
    let w1 = sample_workflow("v1");
    let v1 = lib.register(&kind, &w1).await.expect("register v1");
    assert_eq!(v1, 1);

    let after_v1 = lib
        .lookup(&kind)
        .await
        .expect("lookup after v1")
        .expect("workflow exists");
    assert_eq!(after_v1.nodes.len(), 1);
    assert_eq!(after_v1.edges.len(), 2);
    assert_eq!(after_v1.edges[0].artifact_id, w1.edges[0].artifact_id);

    // Register v2 — latest now wins.
    let w2 = sample_workflow("v2");
    let v2 = lib.register(&kind, &w2).await.expect("register v2");
    assert_eq!(v2, 2);

    let after_v2 = lib
        .latest(&kind)
        .await
        .expect("latest after v2")
        .expect("workflow exists");
    assert_eq!(after_v2.edges[0].artifact_id, w2.edges[0].artifact_id);

    // lookup() and latest() agree.
    let by_lookup = lib.lookup(&kind).await.unwrap().unwrap();
    assert_eq!(by_lookup.edges[0].artifact_id, w2.edges[0].artifact_id);

    cleanup(&pool, &kind).await;
}

#[tokio::test]
async fn lookup_returns_none_for_unknown_kind() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_unknown");

    let lib = WorkflowLibrary::new(pool);
    assert!(lib.lookup(&kind).await.unwrap().is_none());
    assert!(lib.latest(&kind).await.unwrap().is_none());
}

#[tokio::test]
async fn workflow_roundtrips_through_register_and_lookup() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_roundtrip");
    cleanup(&pool, &kind).await;

    let lib = WorkflowLibrary::new(pool.clone());

    let original = sample_workflow("roundtrip");
    let original_json = serde_json::to_value(&original).unwrap();

    lib.register(&kind, &original).await.expect("register");

    let fetched = lib
        .lookup(&kind)
        .await
        .expect("lookup")
        .expect("workflow exists");
    let fetched_json = serde_json::to_value(&fetched).unwrap();

    // Identical JSON ⇒ identical struct — the substrate's serde
    // round-trip property (pinned by `workflow.rs::tests::
    // workflow_roundtrips_through_serde_json`) carries through the
    // database layer.
    assert_eq!(original_json, fetched_json);

    cleanup(&pool, &kind).await;
}

#[tokio::test]
async fn duplicate_kind_version_surfaces_as_error() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_dupe");
    cleanup(&pool, &kind).await;

    let lib = WorkflowLibrary::new(pool.clone());

    // Land version 1 normally.
    let w1 = sample_workflow("dupe_v1");
    let v1 = lib.register(&kind, &w1).await.expect("register v1");
    assert_eq!(v1, 1);

    // Manufacture a colliding write at (kind, 1) directly — the
    // unique constraint on (spec_kind, version) is what
    // `DuplicateKind` maps in the public API. Doing this via raw SQL
    // avoids depending on a race for determinism.
    let json = serde_json::to_value(sample_workflow("dupe_v1_again")).unwrap();
    let direct_err = sqlx::query(
        "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
         VALUES ($1, $2, 1, $3)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&kind)
    .bind(&json)
    .execute(&pool)
    .await
    .expect_err("second insert at (kind, 1) must violate UNIQUE");

    match direct_err {
        sqlx::Error::Database(db_err) => assert!(
            db_err.is_unique_violation(),
            "expected unique violation, got: {db_err}"
        ),
        other => panic!("expected Database error, got: {other:?}"),
    }

    cleanup(&pool, &kind).await;
}

/// Two `register` calls racing on the same fresh kind should produce
/// either one success per call (Postgres serialized them) OR one
/// success and one [`WorkflowLibraryError::DuplicateKind`] (the
/// next-version computation collided). Either outcome proves the
/// "one active per (kind, version)" invariant holds — the table
/// never ends up with two rows at the same version.
#[tokio::test]
async fn concurrent_register_preserves_uniqueness() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_concurrent");
    cleanup(&pool, &kind).await;

    let lib_a = WorkflowLibrary::new(pool.clone());
    let lib_b = WorkflowLibrary::new(pool.clone());
    let kind_a = kind.clone();
    let kind_b = kind.clone();
    let w_a = sample_workflow("race_a");
    let w_b = sample_workflow("race_b");

    let (r_a, r_b) = tokio::join!(
        async move { lib_a.register(&kind_a, &w_a).await },
        async move { lib_b.register(&kind_b, &w_b).await },
    );

    match (&r_a, &r_b) {
        (Ok(va), Ok(vb)) => {
            assert_ne!(
                va, vb,
                "two successful registers must hand out distinct versions"
            );
        }
        (Err(WorkflowLibraryError::DuplicateKind { kind: k, .. }), Ok(_))
        | (Ok(_), Err(WorkflowLibraryError::DuplicateKind { kind: k, .. })) => {
            assert_eq!(k, &kind);
        }
        other => panic!("unexpected concurrent register result: {other:?}"),
    }

    // Final state: at most two rows at distinct versions, both for `kind`.
    let rows: Vec<(i32,)> =
        sqlx::query_as("SELECT version FROM workflow_library WHERE spec_kind = $1")
            .bind(&kind)
            .fetch_all(&pool)
            .await
            .unwrap();
    let mut versions: Vec<i32> = rows.into_iter().map(|(v,)| v).collect();
    versions.sort();
    assert!(
        versions == vec![1] || versions == vec![1, 2],
        "got: {versions:?}"
    );

    cleanup(&pool, &kind).await;
}

#[test]
fn duplicate_kind_error_displays_kind_and_version() {
    let err = WorkflowLibraryError::DuplicateKind {
        kind: "design".into(),
        version: 7,
    };
    let msg = err.to_string();
    assert!(msg.contains("design"), "got: {msg}");
    assert!(msg.contains('7'), "got: {msg}");
}
