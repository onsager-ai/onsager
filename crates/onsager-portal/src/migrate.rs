//! Portal-owned migrations.
//!
//! Runs at server startup. The portal owns three tables:
//!
//! - `factory_tasks` — backlog rows materialized from spec-labeled issues.
//! - `pr_gate_verdicts` — one row per `(pr_artifact_id, head_sha)` for gate
//!   evaluation idempotency.
//! - `pr_branch_links` — per-session branch hint, used to attach
//!   `vertical_lineage` when the PR webhook arrives.
//!
//! Tables managed elsewhere (tenants / projects / installations / events /
//! events_ext / artifacts / vertical_lineage) are not touched here.

use sqlx::postgres::PgPool;

/// Apply all portal-owned table migrations. Idempotent — safe to call on
/// every startup.
pub async fn run(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS factory_tasks (
            id          TEXT        PRIMARY KEY,
            project_id  TEXT        NOT NULL,
            source      TEXT        NOT NULL DEFAULT 'manual',
            source_ref  TEXT        NOT NULL,
            title       TEXT        NOT NULL,
            body        TEXT,
            state       TEXT        NOT NULL DEFAULT 'queued',
            created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (project_id, source_ref)
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_factory_tasks_project ON factory_tasks (project_id)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_factory_tasks_state ON factory_tasks (state)")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pr_gate_verdicts (
            pr_artifact_id TEXT        NOT NULL,
            head_sha       TEXT        NOT NULL,
            verdict        TEXT        NOT NULL,
            recorded_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (pr_artifact_id, head_sha)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pr_branch_links (
            session_id  TEXT        PRIMARY KEY,
            project_id  TEXT,
            branch      TEXT        NOT NULL,
            pr_number   BIGINT,
            recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_pr_branch_links_lookup \
         ON pr_branch_links (project_id, branch, recorded_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}
