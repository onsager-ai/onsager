//! Scheduler-owned table bootstrap.
//!
//! The substrate scheduler persists run state in three spine tables
//! (`execution_plans`, `execution_plan_nodes`, `execution_plan_artifacts`
//! — #499 / #500). Their canonical schema is the spine migration
//! `crates/onsager-spine/migrations/006_execution_plans.sql`, applied in
//! production by the migration runner that owns the spine schema.
//!
//! The scheduler host connects via [`onsager_spine::EventStore`], which
//! does **not** run migrations. So this module ensures the tables exist
//! idempotently at startup by replaying the same canonical `.sql`
//! (inlined via `include_str!` — no DDL drift). On a fresh deploy
//! whichever process migrates first wins; the `CREATE TABLE IF NOT
//! EXISTS` shape makes the loser a no-op. Mirrors the stiglab
//! `run_migrations` fallback pattern.

use sqlx::PgPool;

const EXECUTION_PLANS_SQL: &str =
    include_str!("../../onsager-spine/migrations/006_execution_plans.sql");

/// Ensure the scheduler's durable-state tables exist. Idempotent —
/// safe to call on every startup.
pub async fn run(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(EXECUTION_PLANS_SQL)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("scheduler execution-plan migration failed: {e}"))?;
    Ok(())
}
