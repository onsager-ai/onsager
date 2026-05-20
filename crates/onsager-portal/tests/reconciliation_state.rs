//! Integration test for the spec #121 reconciliation foundation.
//!
//! Pins three things end-to-end against a real Postgres:
//!
//!   1. The `adapter_reconciliation_state` table round-trips: a fresh
//!      load returns a default-shaped state row, an upsert advances
//!      the cursor, and a subsequent load returns the advanced row.
//!   2. The `events_ext (adapter_id, external_ref)` partial unique
//!      index is enforced: two emits with the same key collide on
//!      the second insert. This is the spine's dedup contract — the
//!      idempotency between the webhook path and the poller path
//!      lives on this index, not on application-level coordination.
//!   3. The `projects.ingestion_mode` CHECK constraint accepts the
//!      three documented values and rejects anything else.
//!
//! Skipped when `DATABASE_URL` is unset — the contract lives in the
//! spine schema, which only the Postgres-backed harness exercises.

use onsager_github::AdapterReconciliationState;
use onsager_portal::reconciliation::{load_state, upsert_state};
use sqlx::PgPool;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

#[tokio::test]
async fn reconciliation_state_round_trips() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_portal::migrate::run(&pool).await.expect("migrate");

    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let adapter_id = "github";
    let resource_kind = "issue";

    // Fresh load: no row → default-shaped state, all cursor fields
    // None. The caller can pass this directly into poll_since.
    let initial = load_state(&pool, adapter_id, &workspace_id, resource_kind)
        .await
        .expect("load fresh");
    assert_eq!(initial.adapter_id, adapter_id);
    assert_eq!(initial.workspace_id, workspace_id);
    assert_eq!(initial.resource_kind, resource_kind);
    assert!(initial.last_seen_external_id.is_none());
    assert!(initial.last_seen_updated_at.is_none());
    assert!(initial.etag.is_none());

    // Advance the cursor.
    let advanced = AdapterReconciliationState {
        adapter_id: adapter_id.to_string(),
        workspace_id: workspace_id.clone(),
        resource_kind: resource_kind.to_string(),
        last_seen_external_id: Some("42".to_string()),
        last_seen_updated_at: Some(chrono::Utc::now()),
        etag: Some(r#""abc123""#.to_string()),
    };
    upsert_state(&pool, &advanced).await.expect("upsert");

    // Reload — cursor should reflect the advance, NOT the default.
    let loaded = load_state(&pool, adapter_id, &workspace_id, resource_kind)
        .await
        .expect("load advanced");
    assert_eq!(loaded.last_seen_external_id.as_deref(), Some("42"));
    assert!(loaded.last_seen_updated_at.is_some());
    assert_eq!(loaded.etag.as_deref(), Some(r#""abc123""#));

    // Cleanup.
    sqlx::query("DELETE FROM adapter_reconciliation_state WHERE workspace_id = $1")
        .bind(&workspace_id)
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn events_ext_dedup_enforced_by_partial_unique_index() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_portal::migrate::run(&pool).await.expect("migrate");

    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let adapter_id = "github";
    // Use a UUID-suffixed external_ref so the test is rerunnable
    // without leftover rows colliding.
    let external_ref = format!("github:project:test:issue:{}", Uuid::new_v4());

    // First insert: succeeds.
    let first = sqlx::query(
        r#"
        INSERT INTO events_ext (
            stream_id, namespace, event_type, data, metadata,
            workspace_id, adapter_id, external_ref
        ) VALUES ($1, 'git', 'code.issue_updated', '{}'::jsonb, '{}'::jsonb,
                  $2, $3, $4)
        "#,
    )
    .bind(format!("stream-{}", Uuid::new_v4()))
    .bind(&workspace_id)
    .bind(adapter_id)
    .bind(&external_ref)
    .execute(&pool)
    .await;
    assert!(first.is_ok(), "first insert should succeed: {first:?}");

    // Second insert with the same (adapter_id, external_ref):
    // the partial unique index rejects it. This is the load-bearing
    // dedup property — webhook-arrives-first and poller-arrives-first
    // both produce the same key; whichever wins, the loser is a
    // silent no-op, NOT a duplicate spine row.
    let second = sqlx::query(
        r#"
        INSERT INTO events_ext (
            stream_id, namespace, event_type, data, metadata,
            workspace_id, adapter_id, external_ref
        ) VALUES ($1, 'git', 'code.issue_updated', '{}'::jsonb, '{}'::jsonb,
                  $2, $3, $4)
        "#,
    )
    .bind(format!("stream-{}", Uuid::new_v4()))
    .bind(&workspace_id)
    .bind(adapter_id)
    .bind(&external_ref)
    .execute(&pool)
    .await;
    assert!(
        second.is_err(),
        "duplicate (adapter_id, external_ref) must be rejected by the partial unique index"
    );

    // Cleanup.
    sqlx::query("DELETE FROM events_ext WHERE workspace_id = $1")
        .bind(&workspace_id)
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn ingestion_mode_check_constraint_rejects_unknown_values() {
    let Some(pool) = try_pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    onsager_portal::migrate::run(&pool).await.expect("migrate");

    // Need a workspace row so the FK-less projects.workspace_id is
    // at least populated with a plausible value.
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, 'reconcile-test', 'system', NOW()::text) \
         ON CONFLICT DO NOTHING",
    )
    .bind(&workspace_id)
    .bind(format!("recon-{workspace_id}"))
    .execute(&pool)
    .await
    .expect("seed workspace");

    for mode in ["webhook+reconciler", "polling-only", "webhook-only"] {
        let project_id = format!("proj-{}", Uuid::new_v4());
        let ok = sqlx::query(
            "INSERT INTO projects \
                (id, workspace_id, github_app_installation_id, \
                 repo_owner, repo_name, default_branch, created_at, \
                 ingestion_mode) \
             VALUES ($1, $2, '0', 'owner', 'repo', 'main', NOW()::text, $3)",
        )
        .bind(&project_id)
        .bind(&workspace_id)
        .bind(mode)
        .execute(&pool)
        .await;
        assert!(ok.is_ok(), "mode {mode} should be accepted: {ok:?}");
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(&project_id)
            .execute(&pool)
            .await
            .expect("cleanup project");
    }

    // Anything else must be rejected.
    let project_id = format!("proj-{}", Uuid::new_v4());
    let bad = sqlx::query(
        "INSERT INTO projects \
            (id, workspace_id, github_app_installation_id, \
             repo_owner, repo_name, default_branch, created_at, \
             ingestion_mode) \
         VALUES ($1, $2, '0', 'owner', 'repo', 'main', NOW()::text, 'never-heard-of-it')",
    )
    .bind(&project_id)
    .bind(&workspace_id)
    .execute(&pool)
    .await;
    assert!(
        bad.is_err(),
        "unknown ingestion_mode value must be rejected by CHECK constraint"
    );

    // Cleanup workspace.
    sqlx::query("DELETE FROM workspaces WHERE id = $1")
        .bind(&workspace_id)
        .execute(&pool)
        .await
        .ok();
}
