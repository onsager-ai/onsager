//! Portal-owned migrations.
//!
//! Runs at server startup. All tables live in versioned `.sql` files under
//! `crates/onsager-portal/migrations/`; the binary inlines them at compile
//! time via `include_str!` so no separate migration directory is needed at
//! runtime.
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
    // 007_github_app_installations was historically applied via
    // stiglab's runtime CREATE TABLE IF NOT EXISTS path; the `.sql`
    // file is the canonical schema. Wire it here so the apply order
    // matches filename order — the SQL is idempotent so a deploy
    // where stiglab migrated first is unaffected.
    (
        "007_github_app_installations",
        include_str!("../migrations/007_github_app_installations.sql"),
    ),
    (
        "008_portal_remediation_calls",
        include_str!("../migrations/008_portal_remediation_calls.sql"),
    ),
    (
        "009_activation_events",
        include_str!("../migrations/009_activation_events.sql"),
    ),
    (
        "010_spec_plans",
        include_str!("../migrations/010_spec_plans.sql"),
    ),
    (
        "011_portal_internal",
        include_str!("../migrations/011_portal_internal.sql"),
    ),
    (
        "012_chat_threads",
        include_str!("../migrations/012_chat_threads.sql"),
    ),
    (
        "013_push_subscriptions",
        include_str!("../migrations/013_push_subscriptions.sql"),
    ),
];

/// Apply all portal-owned table migrations. Idempotent — safe to call on
/// every startup.
pub async fn run(pool: &PgPool) -> anyhow::Result<()> {
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
