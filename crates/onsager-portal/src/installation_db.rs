//! GitHub App installation CRUD against the portal-owned
//! `github_app_installations` table (portal migration 007).
//!
//! Spec #222 Slice 3b moved the routes and schema from stiglab to
//! portal. Stiglab still reads the same Postgres table from a separate
//! connection pool for the in-process needs of `routes/projects.rs`
//! live-data hydration, but **portal is the only writer**.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;

use crate::installation::{GitHubAccountType, GitHubAppInstallation};

#[derive(sqlx::FromRow)]
struct InstallationRow {
    id: String,
    workspace_id: String,
    install_id: i64,
    account_login: String,
    account_type: String,
    created_at: String,
}

impl TryFrom<InstallationRow> for GitHubAppInstallation {
    type Error = anyhow::Error;

    fn try_from(row: InstallationRow) -> anyhow::Result<Self> {
        let account_type = GitHubAccountType::parse(&row.account_type).ok_or_else(|| {
            anyhow::anyhow!("invalid github account type stored: {}", row.account_type)
        })?;
        Ok(GitHubAppInstallation {
            id: row.id,
            workspace_id: row.workspace_id,
            install_id: row.install_id,
            account_login: row.account_login,
            account_type,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

pub async fn insert_installation(
    pool: &PgPool,
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

pub async fn list_installations_for_workspace(
    pool: &PgPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<GitHubAppInstallation>> {
    let rows = sqlx::query_as::<_, InstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_installation(
    pool: &PgPool,
    install_row_id: &str,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, InstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE id = $1",
    )
    .bind(install_row_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an installation by its **numeric GitHub install_id** (not
/// the internal UUID). Used by the install callback to detect
/// idempotent re-runs vs. cross-workspace linkage conflicts before
/// inserting.
pub async fn get_installation_by_install_id(
    pool: &PgPool,
    install_id: i64,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, InstallationRow>(
        "SELECT id, workspace_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE install_id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn delete_installation(pool: &PgPool, install_row_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM github_app_installations WHERE id = $1")
        .bind(install_row_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Look up the webhook-secret cipher for a given numeric install id.
///
/// Outer `Option` distinguishes installation presence from cipher
/// presence:
/// - `None` — no row for this `install_id` (unknown installation).
/// - `Some(None)` — row exists, no per-install cipher stored.
/// - `Some(Some(cipher))` — row exists with a per-install cipher.
///
/// Portal's webhook receiver fails closed on a NULL cipher (401 — see
/// `handlers/webhook.rs`). Workflow activation feeds the returned value
/// straight into `ensure_webhook_registered`'s `secret` parameter, so a
/// `Some(None)` install today produces a webhook with no signing secret
/// whose deliveries the receiver will then reject. Surfacing that as an
/// activation-time error is tracked separately — callers should treat
/// `Some(None)` as "not safe to activate."
pub async fn get_install_webhook_secret_cipher(
    pool: &PgPool,
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

/// Count projects that still reference a given installation. Used by
/// the delete-installation route for an app-layer referential-integrity
/// check (the schema does not declare FK constraints, in keeping with
/// the rest of the stiglab/spine schema).
pub async fn count_projects_for_installation(
    pool: &PgPool,
    install_row_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE github_app_installation_id = $1")
            .bind(install_row_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Membership check — used by every workspace-scoped installation
/// route's `require_workspace_access` helper. Reads `workspace_members`
/// from the same Postgres instance (still owned by stiglab's runtime
/// migrations until #222 Slice 3a moves the schema into the spine).
pub async fn is_workspace_member(
    pool: &PgPool,
    workspace_id: &str,
    user_id: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}
