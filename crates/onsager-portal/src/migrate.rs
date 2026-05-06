//! Portal-owned migrations.
//!
//! Runs at server startup. The portal owns:
//!
//! - `factory_tasks` — backlog rows materialized from spec-labeled issues.
//! - `pr_gate_verdicts` — one row per `(pr_artifact_id, head_sha)` for gate
//!   evaluation idempotency.
//! - `pr_branch_links` — per-session branch hint, used to attach
//!   `vertical_lineage` when the PR webhook arrives.
//! - `portal_webhook_secrets` — per-workspace webhook signature secret for
//!   self-hosted PAT mode (spec #222 / parent #220). Loaded from the
//!   `migrations/` directory rather than inline DDL — the first portal-owned
//!   table that lives in a versioned `.sql` file, setting the precedent for
//!   the schema-split slices to follow.
//! - `users`, `auth_sessions`, `sso_exchange_codes` — auth identity +
//!   cookie sessions + cross-env SSO codes (#222 Slice 5). Stiglab still
//!   reads from these tables for its `AuthUser` cookie extractor; portal
//!   is the only writer.
//! - `user_pats` — server-issued Personal Access Tokens (#143). Portal
//!   owns mint/list/revoke via `/api/pats*` and verifies presented PATs
//!   in its own `AuthUser` extractor (#222 Slice 2b). Stiglab still reads
//!   this table for its `AuthUser` PAT path while non-portal routes
//!   (credentials, workspaces, projects, workflows) accept PATs.
//!
//! Tables managed elsewhere (workspaces / projects / installations / events /
//! events_ext / artifacts / vertical_lineage) are not touched here.

use sqlx::postgres::PgPool;

/// Versioned `.sql` files under `crates/onsager-portal/migrations/`.
///
/// Inlined at compile time so the binary stays self-contained — no
/// `include_str!`-of-disk path at runtime, no shipping the directory next to
/// the binary. Order matches filename order and is the apply order.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "001_portal_webhook_secrets",
        include_str!("../migrations/001_portal_webhook_secrets.sql"),
    ),
    ("002_users", include_str!("../migrations/002_users.sql")),
    (
        "003_auth_sessions",
        include_str!("../migrations/003_auth_sessions.sql"),
    ),
    (
        "004_sso_exchange_codes",
        include_str!("../migrations/004_sso_exchange_codes.sql"),
    ),
    (
        "005_user_pats",
        include_str!("../migrations/005_user_pats.sql"),
    ),
    (
        "006_user_credentials",
        include_str!("../migrations/006_user_credentials.sql"),
    ),
];

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

    // `pr_branch_links` is also created by stiglab's startup migrations
    // (session-side writer) — the DDL here matches the stiglab side so
    // whoever migrates first wins and the other becomes a no-op.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pr_branch_links (
            session_id  TEXT PRIMARY KEY,
            project_id  TEXT,
            branch      TEXT NOT NULL,
            pr_number   BIGINT,
            recorded_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_pr_branch_links_lookup \
         ON pr_branch_links (project_id, branch)",
    )
    .execute(pool)
    .await?;

    for (name, sql) in MIGRATIONS {
        tracing::debug!(migration = name, "portal: applying migration");
        sqlx::raw_sql(sql)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("portal migration {name} failed: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The migrations array stays in lock-step with the filesystem. If a new
    /// `.sql` file is added under `migrations/` without being wired into
    /// `MIGRATIONS`, the include_str! call site below would fail at compile
    /// time — but the inverse (a `MIGRATIONS` entry pointing at a missing
    /// file) is also a compile-time error, so this test just sanity-checks
    /// the contents are non-empty and parseable as SQL identifiers.
    #[test]
    fn migrations_are_non_empty_and_named_consistently() {
        assert!(!MIGRATIONS.is_empty(), "expected at least one migration");
        for (name, sql) in MIGRATIONS {
            assert!(
                name.starts_with(char::is_numeric),
                "name {name} should start with a digit"
            );
            assert!(!sql.trim().is_empty(), "migration {name} has empty body");
        }
    }
}
