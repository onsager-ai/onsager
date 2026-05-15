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

/// Drive `register` through a deterministic collision on (kind, 1)
/// so we observe its error-mapping path — not just the raw database
/// constraint. The pattern: open a transaction that inserts at v=1
/// but doesn't commit; in parallel, call `register` (which under
/// READ COMMITTED can't see the uncommitted row, so its `MAX+1`
/// also picks v=1 and its INSERT blocks on the unique constraint);
/// then commit the blocker, which forces `register`'s INSERT to
/// fail with the kind+version unique violation, which `register`
/// must map to `WorkflowLibraryError::DuplicateKind`.
#[tokio::test]
async fn register_maps_kind_version_collision_to_duplicate_kind() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_register_dupe");
    cleanup(&pool, &kind).await;

    // Blocker transaction: holds an uncommitted row at v=1.
    let mut blocker = pool.begin().await.expect("begin blocker tx");
    let blocker_json = serde_json::to_value(sample_workflow("blocker")).unwrap();
    sqlx::query(
        "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
         VALUES ($1, $2, 1, $3)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&kind)
    .bind(&blocker_json)
    .execute(&mut *blocker)
    .await
    .expect("blocker insert");

    // Spawn `register` from outside the blocker's transaction. Its
    // SELECT MAX runs under a separate connection that sees no
    // committed rows for this kind, computes `next = 1`, and tries
    // to INSERT at (kind, 1) — which blocks on the unique constraint
    // until the blocker commits.
    let lib = WorkflowLibrary::new(pool.clone());
    let kind_for_task = kind.clone();
    let racer_workflow = sample_workflow("racer");
    let racer = tokio::spawn(async move { lib.register(&kind_for_task, &racer_workflow).await });

    // Give the spawned task a moment to enqueue its INSERT and hit
    // the row lock. 200ms is generous on local Postgres; the test
    // remains correct if it's longer.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Releasing the blocker commits its row at v=1; the racer's
    // INSERT is now refused by the unique constraint.
    blocker.commit().await.expect("commit blocker");

    match racer.await.expect("join racer") {
        Err(WorkflowLibraryError::DuplicateKind { kind: k, version }) => {
            assert_eq!(k, kind);
            // The error reports whatever version is now live — i.e.
            // the blocker's v=1.
            assert_eq!(version, 1);
        }
        other => panic!("expected DuplicateKind through register, got: {other:?}"),
    }

    cleanup(&pool, &kind).await;
}

/// An unrelated unique-constraint violation (the primary key on
/// `id`) must NOT be reported as `DuplicateKind`. This pins
/// `register`'s constraint-name check so future schema additions
/// (more unique constraints on the table) don't get silently
/// misclassified.
#[tokio::test]
async fn unrelated_unique_violation_does_not_surface_as_duplicate_kind() {
    let Some(url) = db_url() else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let kind = unique_kind("sub04_pk_dupe");
    cleanup(&pool, &kind).await;

    // Take a row id and pre-insert at v=99 to force a PK collision
    // by re-inserting the same id from a separate path. We don't
    // have a public hook to control `register`'s generated id, so we
    // instead exercise the constraint directly and verify the error
    // shape — confirming that `is_unique_violation()` alone would
    // misclassify it.
    let fixed_id = uuid::Uuid::new_v4().to_string();
    let json = serde_json::to_value(sample_workflow("pk_dupe")).unwrap();

    sqlx::query(
        "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
         VALUES ($1, $2, 1, $3)",
    )
    .bind(&fixed_id)
    .bind(&kind)
    .bind(&json)
    .execute(&pool)
    .await
    .expect("seed row");

    let err = sqlx::query(
        "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
         VALUES ($1, $2, 2, $3)",
    )
    .bind(&fixed_id)
    .bind(&kind)
    .bind(&json)
    .execute(&pool)
    .await
    .expect_err("duplicate id must violate PK");

    match err {
        sqlx::Error::Database(db_err) => {
            assert!(
                db_err.is_unique_violation(),
                "expected unique violation, got: {db_err}"
            );
            // The constraint name on the PK is *not* the kind+version
            // one — so register's match-guard would correctly skip it.
            assert_ne!(
                db_err.constraint(),
                Some("workflow_library_kind_version_unique"),
                "PK violation must not look like kind+version collision"
            );
        }
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
