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

use chrono::{DateTime, Utc};
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
}

/// Look up the artifact previously upserted for `(project_id, pr_number)`.
/// Returns `None` for the very first webhook event on a PR.
pub async fn find_pr_artifact(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
) -> anyhow::Result<Option<PrArtifactRow>> {
    let external_ref = pr_external_ref(project_id, pr_number);
    let row: Option<(String, i32)> = sqlx::query_as(
        "SELECT artifact_id, current_version FROM artifacts WHERE external_ref = $1",
    )
    .bind(&external_ref)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(artifact_id, current_version)| PrArtifactRow {
        artifact_id,
        current_version,
    }))
}

/// Insert (or fetch) the canonical PR artifact for `(project_id, pr_number)`.
/// First-write wins; subsequent calls return the existing row. Idempotency is
/// anchored on `external_ref` (the spine's `artifacts.external_ref` is not
/// `UNIQUE` workspace-wide, so we serialize concurrent upserts through a
/// transaction-scoped advisory lock keyed by the ref).
///
/// On a hit we also refresh `name` / `owner` / `state` so the artifact row
/// reflects the latest webhook payload — otherwise a PR retitle or author
/// change never propagates to the dashboard.
pub async fn upsert_pr_artifact(
    pool: &PgPool,
    project_id: &str,
    pr_number: u64,
    name: &str,
    owner: &str,
    state: PrLifecycleState,
) -> anyhow::Result<PrArtifactRow> {
    let external_ref = pr_external_ref(project_id, pr_number);
    let new_id = format!("art_pr_{}", uuid::Uuid::new_v4().simple());
    let artifact_state = state.as_artifact_state();

    let mut tx = pool.begin().await?;

    // Serialize upserts for the same external_ref. Needed because the spine
    // schema does not enforce UNIQUE(external_ref) (artifact_adapters across
    // subsystems may legitimately share the column), so we can't let the
    // DB dedup for us via ON CONFLICT.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&external_ref)
        .execute(&mut *tx)
        .await?;

    let row: (String, i32) = if let Some(existing) = sqlx::query_as::<_, (String, i32)>(
        "UPDATE artifacts \
            SET name = $2, owner = $3, state = $4 \
          WHERE external_ref = $1 \
      RETURNING artifact_id, current_version",
    )
    .bind(&external_ref)
    .bind(name)
    .bind(owner)
    .bind(artifact_state)
    .fetch_optional(&mut *tx)
    .await?
    {
        existing
    } else {
        sqlx::query_as(
            "INSERT INTO artifacts \
                (artifact_id, kind, name, owner, created_by, state, current_version, \
                 external_ref, workspace_id, metadata) \
             VALUES ($1, 'pull_request', $2, $3, 'onsager-portal', $4, 1, $5, $6, \
                     jsonb_build_object('project_id', $6::text, 'pr_number', $7::bigint)) \
             RETURNING artifact_id, current_version",
        )
        .bind(&new_id)
        .bind(name)
        .bind(owner)
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
    })
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

/// Stable external reference key. `project_id` plus `pr_number` is the
/// (project, PR) identity, so the same PR number across two projects never
/// collides.
pub fn pr_external_ref(project_id: &str, pr_number: u64) -> String {
    format!("github:project:{project_id}:pr:{pr_number}")
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

// ── Factory tasks (portal-owned) ──────────────────────────────────────────

/// A factory task row materialized from a webhook event (or, eventually, a
/// dashboard form). `state` follows a small lifecycle: `queued → spawned →
/// closed`. v1 only ever creates `queued` rows.
#[derive(Debug, Clone)]
pub struct FactoryTask {
    pub id: String,
    pub project_id: String,
    pub source: String,
    pub source_ref: String,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub created_at: DateTime<Utc>,
}

/// Insert a `queued` task row, idempotent on `(project_id, source_ref)`.
/// Returns the row that survives (newly inserted or pre-existing).
pub async fn upsert_factory_task(
    pool: &PgPool,
    project_id: &str,
    source: &str,
    source_ref: &str,
    title: &str,
    body: Option<&str>,
) -> anyhow::Result<FactoryTask> {
    let id = format!("ftk_{}", uuid::Uuid::new_v4().simple());
    let now = Utc::now();
    let row: (String, String, String, String, String, Option<String>, String, DateTime<Utc>) =
        sqlx::query_as(
            "WITH ins AS (
                 INSERT INTO factory_tasks (id, project_id, source, source_ref, title, body, state, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7) \
                 ON CONFLICT (project_id, source_ref) DO NOTHING \
                 RETURNING id, project_id, source, source_ref, title, body, state, created_at \
             ) \
             SELECT id, project_id, source, source_ref, title, body, state, created_at FROM ins \
             UNION ALL \
             SELECT id, project_id, source, source_ref, title, body, state, created_at FROM factory_tasks \
                 WHERE project_id = $2 AND source_ref = $4 AND NOT EXISTS (SELECT 1 FROM ins) \
             LIMIT 1",
        )
        .bind(&id)
        .bind(project_id)
        .bind(source)
        .bind(source_ref)
        .bind(title)
        .bind(body)
        .bind(now)
        .fetch_one(pool)
        .await?;
    Ok(FactoryTask {
        id: row.0,
        project_id: row.1,
        source: row.2,
        source_ref: row.3,
        title: row.4,
        body: row.5,
        state: row.6,
        created_at: row.7,
    })
}

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
