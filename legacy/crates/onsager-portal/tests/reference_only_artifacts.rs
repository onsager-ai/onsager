//! Umbrella contract test for spec #170 (reference-only external artifacts).
//!
//! The umbrella's invariant is single-sentence: the spine never copies
//! provider-authored fields. For both `Kind::PullRequest` (child #171) and
//! `Kind::GithubIssue` (child #167), `upsert_pr_artifact_ref` /
//! `upsert_issue_artifact_ref` write identity + our derived lifecycle and
//! nothing else — `name` and `owner` stay NULL on every insert and every
//! subsequent state transition. A PR retitle / author transfer / label edit
//! on GitHub does not touch the spine row because there is no field to
//! drift; the dashboard hydrates the live title via the portal proxy
//! (`live_data.rs` + `proxy_cache.rs`) instead of off the artifact row.
//!
//! This test pins that contract end-to-end against a real Postgres so the
//! invariant cannot regress to "we wrote NULL today but a future PR
//! quietly starts populating it again". The PR-side and issue-side cases
//! are kept symmetrical on purpose — the umbrella's whole point is that
//! the two external-origin kinds share one shape.
//!
//! Skipped when `DATABASE_URL` is unset — the contract lives in the spine
//! schema, which only the Postgres-backed harness exercises.

use onsager_portal::db::{
    self, IssueLifecycleState, PrLifecycleState, issue_external_ref, pr_external_ref,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn try_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("spine connect"))
}

async fn fetch_columns(
    spine: &PgPool,
    artifact_id: &str,
) -> (Option<String>, Option<String>, String, i32, Option<String>) {
    let row = sqlx::query(
        "SELECT name, owner, state, current_version, external_ref \
           FROM artifacts WHERE artifact_id = $1",
    )
    .bind(artifact_id)
    .fetch_one(spine)
    .await
    .expect("artifact row");
    (
        row.try_get("name").unwrap(),
        row.try_get("owner").unwrap(),
        row.get("state"),
        row.get("current_version"),
        row.try_get("external_ref").unwrap(),
    )
}

async fn cleanup(spine: &PgPool, artifact_id: &str) {
    let _ = sqlx::query("DELETE FROM artifacts WHERE artifact_id = $1")
        .bind(artifact_id)
        .execute(spine)
        .await;
}

#[tokio::test]
async fn pr_skeleton_never_writes_name_or_owner() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let project_id = format!("proj-{}", Uuid::new_v4());
    let pr_number: u64 = 42;

    // First webhook (pull_request.opened): skeleton row created.
    let created =
        db::upsert_pr_artifact_ref(&spine, &project_id, pr_number, PrLifecycleState::InProgress)
            .await
            .expect("initial upsert");

    let (name, owner, state, version, external_ref) =
        fetch_columns(&spine, &created.artifact_id).await;
    assert!(name.is_none(), "PR skeleton must not store GitHub title");
    assert!(owner.is_none(), "PR skeleton must not store GitHub author");
    assert_eq!(state, "in_progress");
    assert_eq!(version, 1);
    assert_eq!(
        external_ref.as_deref(),
        Some(pr_external_ref(&project_id, pr_number).as_str())
    );

    // pull_request.edited (title rename on GitHub): the umbrella's headline
    // test — the row must not drift even when the upstream renames. The
    // PR webhook handler only re-runs the upsert with the current lifecycle
    // state, so we model that here.
    let renamed =
        db::upsert_pr_artifact_ref(&spine, &project_id, pr_number, PrLifecycleState::InProgress)
            .await
            .expect("re-upsert after rename");
    assert_eq!(
        renamed.artifact_id, created.artifact_id,
        "idempotent on external_ref"
    );

    let (name_after, owner_after, _, _, _) = fetch_columns(&spine, &created.artifact_id).await;
    assert!(
        name_after.is_none(),
        "rename must not surface a title on the spine row"
    );
    assert!(
        owner_after.is_none(),
        "rename must not surface an author on the spine row"
    );

    // pull_request.closed → state flips to released, name/owner stay NULL.
    db::upsert_pr_artifact_ref(&spine, &project_id, pr_number, PrLifecycleState::Released)
        .await
        .expect("close upsert");
    let (name_closed, owner_closed, state_closed, _, _) =
        fetch_columns(&spine, &created.artifact_id).await;
    assert!(name_closed.is_none());
    assert!(owner_closed.is_none());
    assert_eq!(state_closed, "released");

    cleanup(&spine, &created.artifact_id).await;
}

#[tokio::test]
async fn issue_skeleton_never_writes_name_or_owner() {
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let project_id = format!("proj-{}", Uuid::new_v4());
    let issue_number: u64 = 7;

    let created = db::upsert_issue_artifact_ref(
        &spine,
        &project_id,
        issue_number,
        IssueLifecycleState::Draft,
    )
    .await
    .expect("initial upsert");

    let (name, owner, state, version, external_ref) =
        fetch_columns(&spine, &created.artifact_id).await;
    assert!(name.is_none(), "issue skeleton must not store GitHub title");
    assert!(
        owner.is_none(),
        "issue skeleton must not store GitHub author"
    );
    assert_eq!(state, "draft");
    assert_eq!(version, 1);
    assert_eq!(
        external_ref.as_deref(),
        Some(issue_external_ref(&project_id, issue_number).as_str())
    );

    // issues.closed → archived; issues.reopened → draft; no name/author churn.
    db::upsert_issue_artifact_ref(
        &spine,
        &project_id,
        issue_number,
        IssueLifecycleState::Archived,
    )
    .await
    .expect("close upsert");
    let (_, _, state_closed, _, _) = fetch_columns(&spine, &created.artifact_id).await;
    assert_eq!(state_closed, "archived");

    db::upsert_issue_artifact_ref(
        &spine,
        &project_id,
        issue_number,
        IssueLifecycleState::Draft,
    )
    .await
    .expect("reopen upsert");
    let (name_reopen, owner_reopen, state_reopen, _, _) =
        fetch_columns(&spine, &created.artifact_id).await;
    assert!(name_reopen.is_none());
    assert!(owner_reopen.is_none());
    assert_eq!(state_reopen, "draft");

    cleanup(&spine, &created.artifact_id).await;
}

#[tokio::test]
async fn touch_artifact_bumps_version_but_leaves_provider_fields_null() {
    // `issues.edited` / `issues.labeled` flow through `touch_artifact`,
    // which bumps `current_version` + `last_observed_at`. The umbrella's
    // contract is that this path also never writes provider-authored
    // fields — even when GitHub fires an edit/label event, the spine row
    // shape doesn't move.
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let project_id = format!("proj-{}", Uuid::new_v4());
    let issue_number: u64 = 99;

    let created = db::upsert_issue_artifact_ref(
        &spine,
        &project_id,
        issue_number,
        IssueLifecycleState::Draft,
    )
    .await
    .expect("seed");

    let new_version = db::touch_artifact(&spine, &created.artifact_id)
        .await
        .expect("touch");
    assert_eq!(new_version, 2, "touch_artifact bumps current_version");

    let (name, owner, state, version, _) = fetch_columns(&spine, &created.artifact_id).await;
    assert!(name.is_none(), "touch_artifact must not surface a title");
    assert!(owner.is_none(), "touch_artifact must not surface an author");
    assert_eq!(state, "draft", "touch_artifact leaves state alone");
    assert_eq!(version, 2);

    cleanup(&spine, &created.artifact_id).await;
}

#[tokio::test]
async fn external_ref_with_provider_fields_is_rejected_by_schema() {
    // Spec #336: the contract is mechanically enforced by the
    // `artifacts_external_ref_no_provider_fields` CHECK constraint
    // (migration 030). Any future external-integration write path that
    // stuffs a provider-authored title into `name` (or login into
    // `owner`) alongside a non-NULL `external_ref` is rejected at the
    // schema layer, not just by review. This test exercises the raw
    // INSERT path so a future regression — adding an `upsert_*_artifact`
    // that populates `name` — fails loudly even if `upsert_pr_artifact_ref`
    // / `upsert_issue_artifact_ref` are left intact.
    let Some(spine) = try_pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };

    let artifact_id = format!("art_bad_{}", Uuid::new_v4().simple());
    let external_ref = format!("github:project:proj-{}:pr:1", Uuid::new_v4());

    // `external_ref` + non-NULL `name` → CHECK violation.
    let err = sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, workspace_id) \
         VALUES ($1, 'pull_request', 'PR title leak', NULL, 'tester', 'in_progress', 1, \
                 $2, 'ws-test')",
    )
    .bind(&artifact_id)
    .bind(&external_ref)
    .execute(&spine)
    .await
    .expect_err("CHECK constraint must reject denormalized external state");

    let msg = err.to_string();
    assert!(
        msg.contains("artifacts_external_ref_no_provider_fields"),
        "expected CHECK constraint violation, got: {msg}"
    );

    // `external_ref` + non-NULL `owner` → same violation.
    let err = sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             external_ref, workspace_id) \
         VALUES ($1, 'pull_request', NULL, 'octocat', 'tester', 'in_progress', 1, \
                 $2, 'ws-test')",
    )
    .bind(&artifact_id)
    .bind(&external_ref)
    .execute(&spine)
    .await
    .expect_err("CHECK constraint must reject denormalized author too");
    assert!(
        err.to_string()
            .contains("artifacts_external_ref_no_provider_fields"),
        "expected CHECK constraint violation on owner, got: {err}"
    );

    // Internal-origin artifact (no `external_ref`) with `name` + `owner`
    // populated still inserts — the constraint only binds the external case.
    sqlx::query(
        "INSERT INTO artifacts \
            (artifact_id, kind, name, owner, created_by, state, current_version, \
             workspace_id) \
         VALUES ($1, 'code', 'internal-name', 'tester', 'tester', 'draft', 0, \
                 'ws-test')",
    )
    .bind(&artifact_id)
    .execute(&spine)
    .await
    .expect("internal-origin write path is unaffected by the constraint");

    // UPDATE-path: promoting an internal-origin row to external-origin
    // (setting `external_ref` without first nulling `name`/`owner`) is
    // also rejected. Postgres evaluates the CHECK against the row's
    // post-image, so the constraint binds INSERTs and UPDATEs symmetrically.
    // This pins the contract end-to-end: a future write path that
    // back-fills `external_ref` onto an existing row can't smuggle a
    // provider-authored title back in.
    let promote_ref = format!("github:project:proj-{}:pr:2", Uuid::new_v4());
    let err = sqlx::query("UPDATE artifacts SET external_ref = $2 WHERE artifact_id = $1")
        .bind(&artifact_id)
        .bind(&promote_ref)
        .execute(&spine)
        .await
        .expect_err("UPDATE that introduces external_ref alongside name/owner must violate CHECK");
    assert!(
        err.to_string()
            .contains("artifacts_external_ref_no_provider_fields"),
        "expected CHECK constraint violation on UPDATE, got: {err}"
    );

    // The same UPDATE succeeds once `name` / `owner` are nulled in the
    // same statement — the predicate is forward-looking, not historical.
    sqlx::query(
        "UPDATE artifacts SET external_ref = $2, name = NULL, owner = NULL \
         WHERE artifact_id = $1",
    )
    .bind(&artifact_id)
    .bind(&promote_ref)
    .execute(&spine)
    .await
    .expect("UPDATE that nulls name/owner alongside external_ref is allowed");

    cleanup(&spine, &artifact_id).await;
}
