//! Integration test for spec #464 — `GET /api/workspaces/:id/runs`
//! status + time-range filters at the SQL layer.
//!
//! The handler's HTTP plumbing is the same shape the run-detail and
//! workflow-runs routes already cover; what's new and worth pinning is
//! the SQL the four optional query params (`status`, `since`, `until`,
//! plus the existing `workflow_id`) compose into. The test calls
//! `fetch_workspace_run_rows` directly so the assertions are on row
//! identity, not on serialization order.
//!
//! Skipped when `DATABASE_URL` is unset — the artifact ↔ workflow rows
//! live on the spine, which only the Postgres-backed harness exercises.

use chrono::{Duration, Utc};
use onsager_portal::handlers::runs::{
    RunStatusFilter, WorkspaceRunsQuery, fetch_workspace_run_rows,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

/// Insert one workflow-bound artifact with the given state /
/// parked-reason / `updated_at`. Returns the artifact id.
async fn seed_run(
    spine: &PgPool,
    workspace_id: &str,
    workflow_id: &str,
    state: &str,
    parked_reason: Option<&str>,
    updated_at: chrono::DateTime<Utc>,
) -> String {
    let artifact_id = format!("art_{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, workspace_id, \
             workflow_id, current_stage_index, workflow_parked_reason, \
             created_at, updated_at) \
         VALUES ($1, 'code', 'runs-filter-test', 'tester', 'tester', \
                 $2, $3, $4, 0, $5, $6, $6)",
    )
    .bind(&artifact_id)
    .bind(state)
    .bind(workspace_id)
    .bind(workflow_id)
    .bind(parked_reason)
    .bind(updated_at)
    .execute(spine)
    .await
    .unwrap();
    artifact_id
}

async fn cleanup(spine: &PgPool, workspace_id: &str) {
    let _ = sqlx::query("DELETE FROM artifacts WHERE workspace_id = $1")
        .bind(workspace_id)
        .execute(spine)
        .await;
}

/// Common fixture: 4 runs covering every status × 3 timestamps (now,
/// -2d, -10d). Returns `(workspace_id, ids_by_status)`.
async fn seed_status_fixture(
    spine: &PgPool,
) -> (String, std::collections::HashMap<&'static str, String>) {
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let workflow_id = format!("wf_{}", Uuid::new_v4());
    let now = Utc::now();
    let mut ids = std::collections::HashMap::new();
    ids.insert(
        "passed",
        seed_run(spine, &workspace_id, &workflow_id, "released", None, now).await,
    );
    ids.insert(
        "failed",
        seed_run(
            spine,
            &workspace_id,
            &workflow_id,
            "archived",
            None,
            now - Duration::days(2),
        )
        .await,
    );
    ids.insert(
        "blocked",
        seed_run(
            spine,
            &workspace_id,
            &workflow_id,
            "under_review",
            Some("agent_session: stuck"),
            now - Duration::days(2),
        )
        .await,
    );
    ids.insert(
        "pending",
        seed_run(
            spine,
            &workspace_id,
            &workflow_id,
            "under_review",
            None,
            now - Duration::days(10),
        )
        .await,
    );
    (workspace_id, ids)
}

fn empty_query() -> WorkspaceRunsQuery {
    WorkspaceRunsQuery {
        workflow_id: None,
        status: None,
        since: None,
        until: None,
        limit: None,
    }
}

#[tokio::test]
async fn status_filter_returns_only_matching_state() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let (workspace_id, ids) = seed_status_fixture(&spine).await;

    for (status_str, filter) in [
        ("passed", RunStatusFilter::Passed),
        ("failed", RunStatusFilter::Failed),
        ("blocked", RunStatusFilter::Blocked),
        ("pending", RunStatusFilter::Pending),
    ] {
        let q = WorkspaceRunsQuery {
            status: Some(filter),
            ..empty_query()
        };
        let rows = fetch_workspace_run_rows(&spine, &workspace_id, &q)
            .await
            .expect("fetch rows");
        assert_eq!(
            rows.len(),
            1,
            "status={status_str} should return exactly one row"
        );
        assert_eq!(
            rows[0].artifact_id, ids[status_str],
            "status={status_str} returned the wrong row"
        );
    }

    cleanup(&spine, &workspace_id).await;
}

#[tokio::test]
async fn unfiltered_returns_all_workflow_bound_runs() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let (workspace_id, ids) = seed_status_fixture(&spine).await;

    let rows = fetch_workspace_run_rows(&spine, &workspace_id, &empty_query())
        .await
        .expect("fetch rows");
    assert_eq!(rows.len(), 4, "no filter should return all 4 seeded rows");
    let returned: std::collections::HashSet<String> =
        rows.iter().map(|r| r.artifact_id.clone()).collect();
    for id in ids.values() {
        assert!(returned.contains(id), "expected {id} in result set");
    }

    cleanup(&spine, &workspace_id).await;
}

#[tokio::test]
async fn since_filter_clips_old_rows() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let (workspace_id, ids) = seed_status_fixture(&spine).await;

    // since = now - 5 days → excludes the -10d "pending" row.
    let q = WorkspaceRunsQuery {
        since: Some(Utc::now() - Duration::days(5)),
        ..empty_query()
    };
    let rows = fetch_workspace_run_rows(&spine, &workspace_id, &q)
        .await
        .expect("fetch rows");
    let returned: std::collections::HashSet<String> =
        rows.iter().map(|r| r.artifact_id.clone()).collect();
    assert_eq!(rows.len(), 3, "since=-5d should exclude the -10d row");
    assert!(!returned.contains(&ids["pending"]));
    assert!(returned.contains(&ids["passed"]));
    assert!(returned.contains(&ids["failed"]));
    assert!(returned.contains(&ids["blocked"]));

    cleanup(&spine, &workspace_id).await;
}

#[tokio::test]
async fn until_filter_clips_new_rows() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let (workspace_id, ids) = seed_status_fixture(&spine).await;

    // until = now - 1 day → excludes the "passed" row at "now".
    let q = WorkspaceRunsQuery {
        until: Some(Utc::now() - Duration::days(1)),
        ..empty_query()
    };
    let rows = fetch_workspace_run_rows(&spine, &workspace_id, &q)
        .await
        .expect("fetch rows");
    let returned: std::collections::HashSet<String> =
        rows.iter().map(|r| r.artifact_id.clone()).collect();
    assert!(!returned.contains(&ids["passed"]));
    assert!(returned.contains(&ids["failed"]));
    assert!(returned.contains(&ids["blocked"]));
    assert!(returned.contains(&ids["pending"]));

    cleanup(&spine, &workspace_id).await;
}

#[tokio::test]
async fn combined_workflow_id_status_and_time_range() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let workspace_id = format!("ws-{}", Uuid::new_v4());
    let wf_a = format!("wf_{}", Uuid::new_v4());
    let wf_b = format!("wf_{}", Uuid::new_v4());
    let now = Utc::now();

    // wf_a — one passed in window, one passed too old.
    let in_window = seed_run(&spine, &workspace_id, &wf_a, "released", None, now).await;
    let _old = seed_run(
        &spine,
        &workspace_id,
        &wf_a,
        "released",
        None,
        now - Duration::days(10),
    )
    .await;
    // wf_a — passed in window but wrong workflow (should still appear when
    // we filter by wf_a; included to keep the noise representative).
    let _failed_in_window = seed_run(
        &spine,
        &workspace_id,
        &wf_a,
        "archived",
        None,
        now - Duration::hours(2),
    )
    .await;
    // wf_b — passed in window; excluded because workflow_id filter is wf_a.
    let _other_workflow = seed_run(&spine, &workspace_id, &wf_b, "released", None, now).await;

    let q = WorkspaceRunsQuery {
        workflow_id: Some(wf_a.clone()),
        status: Some(RunStatusFilter::Passed),
        since: Some(now - Duration::days(5)),
        until: Some(now + Duration::hours(1)),
        limit: None,
    };
    let rows = fetch_workspace_run_rows(&spine, &workspace_id, &q)
        .await
        .expect("fetch rows");
    assert_eq!(
        rows.len(),
        1,
        "combined filter should narrow to one row, got {rows:?}",
    );
    assert_eq!(rows[0].artifact_id, in_window);
    assert_eq!(rows[0].workflow_id, wf_a);

    cleanup(&spine, &workspace_id).await;
}
