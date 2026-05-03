//! Postgres connectivity and queries shared between the webhook server,
//! migration runner, and backfill CLI.
//!
//! The portal speaks Postgres directly (not the cross-backend `AnyPool`
//! stiglab uses) because it co-locates with the spine, which is Postgres-only,
//! and the only tables the portal authors itself (`factory_tasks`,
//! `pr_gate_verdicts`, `pr_branch_links`) live in the same database.
//!
//! Tables not owned by the portal (tenants, github_app_installations,
//! projects, sessions, artifacts, vertical_lineage) are created by stiglab
//! and onsager-spine; the portal is a read-or-append-only consumer.

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Open a fresh connection pool against `database_url`. Caller is responsible
/// for running portal migrations (`migrate::run`) afterward.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// A row from `github_app_installations` plus its decrypted webhook secret.
#[derive(Debug, Clone)]
pub struct InstallationRecord {
    pub id: String,
    pub tenant_id: String,
    pub install_id: i64,
    pub account_login: String,
    pub webhook_secret_cipher: Option<String>,
}

/// Look up an installation row by its GitHub-side install id (the integer in
/// the webhook payload's `installation.id`).
pub async fn find_installation_by_install_id(
    pool: &PgPool,
    install_id: i64,
) -> anyhow::Result<Option<InstallationRecord>> {
    let row: Option<(String, String, i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, tenant_id, install_id, account_login, webhook_secret_cipher \
         FROM github_app_installations WHERE install_id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(id, tenant_id, install_id, account_login, webhook_secret_cipher)| InstallationRecord {
            id,
            tenant_id,
            install_id,
            account_login,
            webhook_secret_cipher,
        },
    ))
}

/// A project row resolved from `(installation, repo_owner, repo_name)`.
#[derive(Debug, Clone)]
pub struct ProjectRecord {
    pub id: String,
    pub tenant_id: String,
    pub github_app_installation_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub default_branch: String,
}

/// Resolve a (installation, owner/repo) tuple to its project row. Returns
/// `None` when the repo is not opted in for this installation — opt-in is
/// the v1 invariant from spec #59 (`Phase 0 alignment: opt-in per repo`).
pub async fn find_project_for_repo(
    pool: &PgPool,
    installation_id: &str,
    owner: &str,
    name: &str,
) -> anyhow::Result<Option<ProjectRecord>> {
    let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, tenant_id, github_app_installation_id, repo_owner, repo_name, default_branch \
         FROM projects \
         WHERE github_app_installation_id = $1 AND repo_owner = $2 AND repo_name = $3",
    )
    .bind(installation_id)
    .bind(owner)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(id, tenant_id, github_app_installation_id, repo_owner, repo_name, default_branch)| {
            ProjectRecord {
                id,
                tenant_id,
                github_app_installation_id,
                repo_owner,
                repo_name,
                default_branch,
            }
        },
    ))
}

pub async fn get_project(pool: &PgPool, project_id: &str) -> anyhow::Result<Option<ProjectRecord>> {
    let row: Option<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, tenant_id, github_app_installation_id, repo_owner, repo_name, default_branch \
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(id, tenant_id, github_app_installation_id, repo_owner, repo_name, default_branch)| {
            ProjectRecord {
                id,
                tenant_id,
                github_app_installation_id,
                repo_owner,
                repo_name,
                default_branch,
            }
        },
    ))
}

/// PR artifact lookup result.
#[derive(Debug, Clone)]
pub struct PrArtifactRow {
    pub artifact_id: String,
    pub current_version: i32,
    /// Tenant scope copied off the spine row (#183). Callers stamp it
    /// onto every `events_ext` emit so the dashboard's per-workspace
    /// stream sees the event.
    pub workspace_id: String,
}

/// Issue artifact lookup result. Same shape as `PrArtifactRow` but typed
/// separately so callers can't accidentally cross-write — issues live under
/// `Kind::GithubIssue`, PRs under `Kind::PullRequest`.
#[derive(Debug, Clone)]
pub struct IssueArtifactRow {
    pub artifact_id: String,
    pub current_version: i32,
    /// Tenant scope copied off the spine row (#183).
    pub workspace_id: String,
}

/// Look up the artifact previously upserted for `(project_id, pr_number)`.
/// Returns `None` for the very first webhook event on a PR.
pub async fn find_pr_artifact(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
) -> anyhow::Result<Option<PrArtifactRow>> {
    let external_ref = pr_external_ref(project_id, pr_number);
    let row: Option<(String, i32, String)> = sqlx::query_as(
        "SELECT artifact_id, current_version, workspace_id FROM artifacts WHERE external_ref = $1",
    )
    .bind(&external_ref)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(artifact_id, current_version, workspace_id)| PrArtifactRow {
            artifact_id,
            current_version,
            workspace_id,
        },
    ))
}

/// Upsert the canonical reference-only PR artifact for `(project_id, pr_number)`.
///
/// Per spec #170/#171, the spine never copies GitHub-authored fields. The
/// row carries identity (`external_ref`, `kind`, `metadata.pr_number`) and
/// our derived `state` only — `name` / `owner` are written NULL and the
/// dashboard hydrates them via the live `/api/projects/:id/pulls` proxy.
///
/// `last_observed_at` is stamped on every webhook touch so the dashboard
/// can render "last seen N min ago" placeholders when the proxy is rate-
/// limited (#170 fail-open decision).
///
/// Idempotency is anchored on `external_ref`. The spine's column is not
/// `UNIQUE` workspace-wide (artifact_adapters across subsystems may share
/// it legitimately), so concurrent upserts serialize through a transaction-
/// scoped advisory lock keyed by the ref.
pub async fn upsert_pr_artifact_ref(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
    state: PrLifecycleState,
) -> anyhow::Result<PrArtifactRow> {
    let external_ref = pr_external_ref(project_id, pr_number);
    let new_id = format!("art_pr_{}", uuid::Uuid::new_v4().simple());
    let artifact_state = state.as_artifact_state();

    let mut tx = pool.begin().await?;

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&external_ref)
        .execute(&mut *tx)
        .await?;

    let row: (String, i32, String) = if let Some(existing) =
        sqlx::query_as::<_, (String, i32, String)>(
            "UPDATE artifacts \
            SET state = $2, last_observed_at = NOW() \
          WHERE external_ref = $1 \
      RETURNING artifact_id, current_version, workspace_id",
        )
        .bind(&external_ref)
        .bind(artifact_state)
        .fetch_optional(&mut *tx)
        .await?
    {
        existing
    } else {
        sqlx::query_as(
            "INSERT INTO artifacts \
                (artifact_id, kind, name, owner, created_by, state, current_version, \
                 external_ref, workspace_id, metadata, last_observed_at) \
             VALUES ($1, 'pull_request', NULL, NULL, 'onsager-portal', $2, 1, $3, $4, \
                     jsonb_build_object('project_id', $4::text, 'pr_number', $5::bigint), NOW()) \
             RETURNING artifact_id, current_version, workspace_id",
        )
        .bind(&new_id)
        .bind(artifact_state)
        .bind(&external_ref)
        .bind(project_id)
        .bind(pr_number as i64)
        .fetch_one(&mut *tx)
        .await?
    };

    tx.commit().await?;

    Ok(PrArtifactRow {
        artifact_id: row.0,
        current_version: row.1,
        workspace_id: row.2,
    })
}

/// Upsert a reference-only GitHub-issue artifact for `(project_id, issue_number)`.
///
/// Matches `upsert_pr_artifact_ref` in shape — identity + derived state, no
/// provider-authored fields. State maps issue-open → `draft`, issue-closed
/// → `archived`, mirroring the existing artifact-state CHECK enum without
/// introducing new vocabulary.
pub async fn upsert_issue_artifact_ref(
    pool: &PgPool,
    project_id: &str,
    issue_number: u64,
    state: IssueLifecycleState,
) -> anyhow::Result<IssueArtifactRow> {
    let external_ref = issue_external_ref(project_id, issue_number);
    let new_id = format!("art_iss_{}", uuid::Uuid::new_v4().simple());
    let artifact_state = state.as_artifact_state();

    let mut tx = pool.begin().await?;

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&external_ref)
        .execute(&mut *tx)
        .await?;

    let row: (String, i32, String) = if let Some(existing) =
        sqlx::query_as::<_, (String, i32, String)>(
            "UPDATE artifacts \
            SET state = $2, last_observed_at = NOW() \
          WHERE external_ref = $1 \
      RETURNING artifact_id, current_version, workspace_id",
        )
        .bind(&external_ref)
        .bind(artifact_state)
        .fetch_optional(&mut *tx)
        .await?
    {
        existing
    } else {
        sqlx::query_as(
            "INSERT INTO artifacts \
                (artifact_id, kind, name, owner, created_by, state, current_version, \
                 external_ref, workspace_id, metadata, last_observed_at) \
             VALUES ($1, 'github_issue', NULL, NULL, 'onsager-portal', $2, 1, $3, $4, \
                     jsonb_build_object('project_id', $4::text, 'issue_number', $5::bigint), NOW()) \
             RETURNING artifact_id, current_version, workspace_id",
        )
        .bind(&new_id)
        .bind(artifact_state)
        .bind(&external_ref)
        .bind(project_id)
        .bind(issue_number as i64)
        .fetch_one(&mut *tx)
        .await?
    };

    tx.commit().await?;

    Ok(IssueArtifactRow {
        artifact_id: row.0,
        current_version: row.1,
        workspace_id: row.2,
    })
}

/// Bump `current_version` and refresh `last_observed_at` without touching
/// `state`. Used for issue edits / label changes — events that signal
/// activity but don't move the open/closed lifecycle.
pub async fn touch_artifact(pool: &PgPool, artifact_id: &str) -> anyhow::Result<i32> {
    let row: (i32,) = sqlx::query_as(
        "UPDATE artifacts SET current_version = current_version + 1, last_observed_at = NOW() \
         WHERE artifact_id = $1 RETURNING current_version",
    )
    .bind(artifact_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Bump the artifact version and (optionally) transition its lifecycle state.
/// Used on `synchronize` (new commits) and `closed` (merged or abandoned).
pub async fn bump_pr_artifact(
    pool: &PgPool,
    artifact_id: &str,
    new_state: Option<PrLifecycleState>,
) -> anyhow::Result<i32> {
    let row: (i32,) = if let Some(state) = new_state {
        sqlx::query_as(
            "UPDATE artifacts SET current_version = current_version + 1, state = $2 \
             WHERE artifact_id = $1 RETURNING current_version",
        )
        .bind(artifact_id)
        .bind(state.as_artifact_state())
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as(
            "UPDATE artifacts SET current_version = current_version + 1 \
             WHERE artifact_id = $1 RETURNING current_version",
        )
        .bind(artifact_id)
        .fetch_one(pool)
        .await?
    };
    Ok(row.0)
}

/// Stable external reference key for a PR. `project_id` plus `pr_number` is
/// the (project, PR) identity, so the same PR number across two projects
/// never collides.
pub fn pr_external_ref(project_id: &str, pr_number: u64) -> String {
    format!("github:project:{project_id}:pr:{pr_number}")
}

/// Stable external reference key for a GitHub issue. Mirrors `pr_external_ref`.
pub fn issue_external_ref(project_id: &str, issue_number: u64) -> String {
    format!("github:project:{project_id}:issue:{issue_number}")
}

/// Subset of `ArtifactState` the portal cares about for PR rows.
#[derive(Debug, Clone, Copy)]
pub enum PrLifecycleState {
    InProgress,
    UnderReview,
    Released,
    Archived,
}

impl PrLifecycleState {
    fn as_artifact_state(self) -> &'static str {
        match self {
            PrLifecycleState::InProgress => "in_progress",
            PrLifecycleState::UnderReview => "under_review",
            PrLifecycleState::Released => "released",
            PrLifecycleState::Archived => "archived",
        }
    }
}

/// Subset of `ArtifactState` the portal cares about for issue rows. Maps
/// GitHub issue state to the existing artifact-state CHECK enum without
/// introducing new vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueLifecycleState {
    /// Open issue.
    Draft,
    /// Closed issue (any reason).
    Archived,
}

impl IssueLifecycleState {
    fn as_artifact_state(self) -> &'static str {
        match self {
            IssueLifecycleState::Draft => "draft",
            IssueLifecycleState::Archived => "archived",
        }
    }

    /// Map a GitHub `state` string (`"open"` / `"closed"`) to our enum.
    pub fn from_github(state: &str) -> Self {
        match state {
            "closed" => Self::Archived,
            _ => Self::Draft,
        }
    }
}

// ── Factory tasks (legacy) ────────────────────────────────────────────────
//
// The `factory_tasks` table from #60 is no longer written by any path —
// reference-only artifact rows (`Kind::GithubIssue`) now serve the inbox
// concept the table was created for. The migration in `migrate.rs` keeps
// the table present so historical rows remain queryable; the writer
// function was removed when the last caller (issues webhook + backfill)
// switched to ref-only writes per #167.

// ── Verdict dedup (Phase 2) ───────────────────────────────────────────────

/// Returns the existing verdict for `(pr_artifact_id, head_sha)` if any.
/// Used to short-circuit duplicate `synchronize` events on the same SHA so
/// gate evaluation runs at most once per commit.
pub async fn find_existing_verdict(
    pool: &PgPool,
    pr_artifact_id: &str,
    head_sha: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT verdict FROM pr_gate_verdicts \
         WHERE pr_artifact_id = $1 AND head_sha = $2",
    )
    .bind(pr_artifact_id)
    .bind(head_sha)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(v,)| v))
}

/// Persist a verdict for `(pr_artifact_id, head_sha)`. Idempotent — if a row
/// already exists for the SHA the new write is silently dropped.
pub async fn record_verdict(
    pool: &PgPool,
    pr_artifact_id: &str,
    head_sha: &str,
    verdict: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO pr_gate_verdicts (pr_artifact_id, head_sha, verdict, recorded_at) \
         VALUES ($1, $2, $3, NOW()) \
         ON CONFLICT (pr_artifact_id, head_sha) DO NOTHING",
    )
    .bind(pr_artifact_id)
    .bind(head_sha)
    .bind(verdict)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Session↔PR correlation (Phase 1) ──────────────────────────────────────

/// Persist a (session_id, branch) hint emitted at session completion. The
/// portal queries this on `pr.opened` to attach `vertical_lineage` when the
/// PR's `head.ref` matches a session's branch.
pub async fn record_session_branch(
    pool: &PgPool,
    session_id: &str,
    project_id: Option<&str>,
    branch: &str,
    pr_number: Option<u64>,
) -> anyhow::Result<()> {
    // `recorded_at` is TEXT in the shared schema (stiglab's AnyPool writer
    // needs SQLite compatibility); explicit RFC-3339 here so lexicographic
    // ordering in `find_session_for_branch` gives us chronological order.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO pr_branch_links (session_id, project_id, branch, pr_number, recorded_at) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (session_id) DO UPDATE SET branch = EXCLUDED.branch, \
            project_id = EXCLUDED.project_id, pr_number = EXCLUDED.pr_number, \
            recorded_at = EXCLUDED.recorded_at",
    )
    .bind(session_id)
    .bind(project_id)
    .bind(branch)
    .bind(pr_number.map(|n| n as i64))
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find the most recent session that pushed `branch` for `project_id`. Used
/// to attach a session→PR vertical lineage when a webhook arrives.
pub async fn find_session_for_branch(
    pool: &PgPool,
    project_id: &str,
    branch: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT session_id FROM pr_branch_links \
         WHERE project_id = $1 AND branch = $2 \
         ORDER BY recorded_at DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(branch)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(s,)| s))
}

/// Insert a `vertical_lineage` row tying a session to the PR artifact's
/// `current_version`. Safe to call repeatedly — the underlying unique index
/// makes the second insert a no-op.
pub async fn link_session_to_pr_artifact(
    pool: &PgPool,
    artifact_id: &str,
    session_id: &str,
    version: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO vertical_lineage (artifact_id, version, session_id) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (artifact_id, version, session_id) DO NOTHING",
    )
    .bind(artifact_id)
    .bind(version)
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}
