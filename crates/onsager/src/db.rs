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
];

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?)
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
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
