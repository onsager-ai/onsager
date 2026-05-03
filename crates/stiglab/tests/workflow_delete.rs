//! Integration test for `workflow_db::delete_workflow` (#233).
//!
//! Pre-fix, deleting a workflow that still had a parked artifact pointing
//! at it raised an FK violation — `artifacts.workflow_id` had no
//! `ON DELETE` clause in migration 006. Migration 015 switches the FK to
//! `ON DELETE SET NULL` and `delete_workflow` now wraps the cleanup in a
//! transaction that NULLs `current_stage_index` and
//! `workflow_parked_reason` alongside `workflow_id`. This test pins both
//! halves: the workflow row goes away, the artifact survives detached.
//!
//! Skipped when `DATABASE_URL` is unset — the artifact ↔ workflow FK
//! lives on the spine, which only the Postgres-backed harness exercises.

use chrono::Utc;
use sqlx::{PgPool, Row};
use stiglab::core::workflow::{TriggerKind, Workflow, WorkflowStage};
use stiglab::core::GateKind;
use stiglab::server::workflow_db;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

async fn seed_workflow(spine: &PgPool, workspace_id: &str) -> String {
    let now = Utc::now();
    let wf = Workflow {
        id: format!("wf_{}", Uuid::new_v4()),
        workspace_id: workspace_id.to_string(),
        name: "delete-test".into(),
        trigger_kind: TriggerKind::GithubIssueWebhook,
        repo_owner: "acme".into(),
        repo_name: "widgets".into(),
        trigger_label: "ai".into(),
        install_id: 1,
        preset_id: None,
        active: false,
        created_by: "u1".into(),
        created_at: now,
        updated_at: now,
    };
    let stage = WorkflowStage {
        id: Uuid::new_v4().to_string(),
        workflow_id: wf.id.clone(),
        seq: 0,
        gate_kind: GateKind::AgentSession,
        params: serde_json::json!({}),
    };
    let id = wf.id.clone();
    workflow_db::insert_workflow_with_stages(spine, &wf, &[stage])
        .await
        .unwrap();
    id
}

async fn seed_parked_artifact(spine: &PgPool, workflow_id: &str) -> String {
    let artifact_id = format!("art_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, \
             workflow_id, current_stage_index, workflow_parked_reason) \
         VALUES ($1, 'code', 'delete-test', 'tester', 'tester', \
                 'under_review', $2, 0, 'agent_session: stuck')",
    )
    .bind(&artifact_id)
    .bind(workflow_id)
    .execute(spine)
    .await
    .unwrap();
    artifact_id
}

#[tokio::test]
async fn delete_workflow_with_parked_artifact_succeeds() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let workflow_id = seed_workflow(&spine, &workspace_id).await;
    let artifact_id = seed_parked_artifact(&spine, &workflow_id).await;

    workflow_db::delete_workflow(&spine, &workflow_id)
        .await
        .expect("delete should succeed even with a parked artifact");

    // Workflow row + cascading stage rows are gone.
    let wf_count: i64 = sqlx::query("SELECT COUNT(*) FROM workflows WHERE workflow_id = $1")
        .bind(&workflow_id)
        .fetch_one(&spine)
        .await
        .unwrap()
        .get(0);
    assert_eq!(wf_count, 0, "workflow row should be deleted");

    let stage_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM workflow_stages WHERE workflow_id = $1")
            .bind(&workflow_id)
            .fetch_one(&spine)
            .await
            .unwrap()
            .get(0);
    assert_eq!(stage_count, 0, "workflow_stages should cascade-delete");

    // Artifact survives, all workflow tagging cleared.
    let row = sqlx::query(
        "SELECT workflow_id, current_stage_index, workflow_parked_reason, state \
           FROM artifacts WHERE artifact_id = $1",
    )
    .bind(&artifact_id)
    .fetch_one(&spine)
    .await
    .unwrap();
    let workflow_id_after: Option<String> = row.try_get("workflow_id").unwrap();
    let stage_index_after: Option<i32> = row.try_get("current_stage_index").unwrap();
    let parked_reason_after: Option<String> = row.try_get("workflow_parked_reason").unwrap();
    let state_after: String = row.get("state");
    assert!(workflow_id_after.is_none(), "workflow_id should be cleared");
    assert!(
        stage_index_after.is_none(),
        "current_stage_index should be cleared"
    );
    assert!(
        parked_reason_after.is_none(),
        "workflow_parked_reason should be cleared"
    );
    assert_eq!(
        state_after, "under_review",
        "non-workflow artifact columns survive untouched"
    );

    // Cleanup: drop the orphaned artifact so reruns don't accumulate.
    let _ = sqlx::query("DELETE FROM artifacts WHERE artifact_id = $1")
        .bind(&artifact_id)
        .execute(&spine)
        .await;
}

#[tokio::test]
async fn delete_workflow_without_artifacts_still_works() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let workflow_id = seed_workflow(&spine, &workspace_id).await;

    workflow_db::delete_workflow(&spine, &workflow_id)
        .await
        .expect("delete with zero artifacts should succeed");

    let wf_count: i64 = sqlx::query("SELECT COUNT(*) FROM workflows WHERE workflow_id = $1")
        .bind(&workflow_id)
        .fetch_one(&spine)
        .await
        .unwrap()
        .get(0);
    assert_eq!(wf_count, 0);
}
