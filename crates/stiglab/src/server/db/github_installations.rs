use crate::core::GitHubAppInstallation;
use chrono::Utc;
use sqlx::AnyPool;

pub async fn insert_github_app_installation(
    pool: &AnyPool,
    install: &GitHubAppInstallation,
    webhook_secret_cipher: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO github_app_installations (id, workspace_id, install_id, account_login, \
                                               account_type, webhook_secret_cipher, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&install.id)
    .bind(&install.workspace_id)
    .bind(install.install_id)
    .bind(&install.account_login)
    .bind(install.account_type.to_string())
    .bind(webhook_secret_cipher)
    .bind(install.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_github_app_installations_for_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<GitHubAppInstallation>> {
    let rows = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an installation by its **numeric GitHub install_id** (not the
/// internal UUID).  Used by the install callback to detect idempotent
/// re-runs vs. cross-workspace linkage conflicts before inserting.
pub async fn get_github_app_installation_by_install_id(
    pool: &AnyPool,
    install_id: i64,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE install_id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn delete_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM github_app_installations WHERE id = $1")
        .bind(install_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Look up the webhook-secret cipher for a given numeric install id.
///
/// Outer `Option` distinguishes installation presence so the webhook
/// handler can fail closed on genuinely unknown installations while still
/// falling back to the global App secret for installs registered via the
/// OAuth callback (which persist without a cipher):
/// - `None` — no row for this `install_id` (unknown installation).
/// - `Some(None)` — row exists, no per-install cipher stored.
/// - `Some(Some(cipher))` — row exists with a per-install cipher.
pub async fn get_install_webhook_secret_cipher(
    pool: &AnyPool,
    install_id: i64,
) -> anyhow::Result<Option<Option<String>>> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT webhook_secret_cipher FROM github_app_installations WHERE install_id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(c,)| c))
}

/// Count projects that still reference a given installation. Used by the
/// delete-installation route for an app-layer referential-integrity check:
/// the tables do not declare FK constraints (consistent with the rest of
/// stiglab, which uses AnyPool across SQLite/Postgres — SQLite needs
/// `PRAGMA foreign_keys = ON` to enforce FKs and the rest of the schema
/// matches this convention), so callers must gate destructive operations
/// explicitly.
pub async fn count_projects_for_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE github_app_installation_id = $1")
            .bind(install_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

// ── Row types ──

#[derive(sqlx::FromRow)]
struct GitHubAppInstallationRow {
    id: String,
    workspace_id: String,
    install_id: i64,
    account_login: String,
    account_type: String,
    created_at: String,
}

impl TryFrom<GitHubAppInstallationRow> for GitHubAppInstallation {
    type Error = anyhow::Error;

    fn try_from(row: GitHubAppInstallationRow) -> anyhow::Result<Self> {
        Ok(GitHubAppInstallation {
            id: row.id,
            workspace_id: row.workspace_id,
            install_id: row.install_id,
            account_login: row.account_login,
            account_type: row
                .account_type
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}
