//! Pool + boot-time migration runner.
//!
//! Migrations are embedded (`include_str!`), applied in order inside a
//! transaction each, and recorded by filename in `schema_migrations`.
//! Files are immutable once merged — fixes are new migrations. This
//! replaces the legacy re-apply-idempotent-files-every-boot scheme and
//! with it the whole "guard every statement against its own prior
//! effects" class of bugs (legacy #613).

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Ordered list of embedded migrations. Append-only; the pre-commit
/// hook enforces the NNN_ numbering stays contiguous.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "001_identity.sql",
        include_str!("../migrations/001_identity.sql"),
    ),
    (
        "002_run_loop.sql",
        include_str!("../migrations/002_run_loop.sql"),
    ),
    (
        "003_github.sql",
        include_str!("../migrations/003_github.sql"),
    ),
    (
        "004_session_events.sql",
        include_str!("../migrations/004_session_events.sql"),
    ),
    (
        "005_machines.sql",
        include_str!("../migrations/005_machines.sql"),
    ),
];

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?)
}

/// One-shot, opt-in destructive schema reset (#637). When
/// `ONSAGER_RESET_SCHEMA=true`, drop and recreate the `public` schema
/// before migrating. The 0.5 reset (ADR 0029) ships a fresh schema with
/// no v1→v2 migration, so a database that still carries the v1 schema
/// (e.g. the persistent production DB) makes migration 001 collide
/// (`relation "users" already exists`). This is the operator's escape
/// hatch: set the flag once, deploy (drops → migrates → boots clean),
/// then remove the flag. Off by default; loud when it fires.
///
/// Pre-launch affordance — also resets preview/staging DBs. Tightening
/// or removing it is part of the launch posture flip.
pub async fn reset_schema_if_requested(pool: &PgPool) -> anyhow::Result<()> {
    if std::env::var("ONSAGER_RESET_SCHEMA").as_deref() != Ok("true") {
        return Ok(());
    }
    tracing::warn!(
        "ONSAGER_RESET_SCHEMA=true — DROPPING and recreating the public \
         schema before migrating. All existing data is destroyed. Remove \
         the flag after this deploy."
    );
    sqlx::raw_sql(
        "DROP SCHEMA public CASCADE; \
         CREATE SCHEMA public; \
         GRANT ALL ON SCHEMA public TO CURRENT_USER; \
         GRANT ALL ON SCHEMA public TO public;",
    )
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("schema reset failed: {e}"))?;
    tracing::warn!("public schema reset complete");
    Ok(())
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    reset_schema_if_requested(pool).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             filename   TEXT PRIMARY KEY,
             applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
         )",
    )
    .execute(pool)
    .await?;

    for (filename, sql) in MIGRATIONS {
        let applied: Option<(String,)> =
            sqlx::query_as("SELECT filename FROM schema_migrations WHERE filename = $1")
                .bind(filename)
                .fetch_optional(pool)
                .await?;
        if applied.is_some() {
            continue;
        }
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("migration {filename} failed: {e}"))?;
        sqlx::query("INSERT INTO schema_migrations (filename) VALUES ($1)")
            .bind(filename)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(migration = filename, "applied");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::MIGRATIONS;

    /// Every `NNN_*.sql` file under migrations/ must be wired into the
    /// embedded `MIGRATIONS` list — otherwise the file ships but never
    /// runs (the M4 `005_machines.sql` bug, caught by the fleet e2e, not
    /// CI). This guard fails the build if a file is added but not embedded.
    #[test]
    fn every_migration_file_is_embedded() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .expect("read migrations dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        files.sort();
        let embedded: Vec<String> = MIGRATIONS.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(
            files, embedded,
            "migrations/ files must exactly match the embedded MIGRATIONS list"
        );
    }
}
