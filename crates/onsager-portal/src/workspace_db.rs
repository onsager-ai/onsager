//! Workspace / member / project CRUD against the spine-owned tables
//! (`crates/onsager-spine/migrations/020_workspaces_to_spine.sql`).
//!
//! Spec #222 Slice 3a moved the routes from stiglab → portal; the
//! supporting DB functions move with them. Stiglab still reads the same
//! tables from the same Postgres instance (separate connection pool) for
//! its in-process session/task workflow needs, but **portal is the only
//! writer**.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;

use crate::core::{Project, Workspace, WorkspaceMember, WorkspaceMemberWithUser};

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: String,
    slug: String,
    name: String,
    created_by: String,
    created_at: String,
}

impl TryFrom<WorkspaceRow> for Workspace {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceRow) -> anyhow::Result<Self> {
        Ok(Workspace {
            id: row.id,
            slug: row.slug,
            name: row.name,
            created_by: row.created_by,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceMemberWithUserRow {
    workspace_id: String,
    user_id: String,
    joined_at: String,
    github_login: Option<String>,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
}

impl TryFrom<WorkspaceMemberWithUserRow> for WorkspaceMemberWithUser {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceMemberWithUserRow) -> anyhow::Result<Self> {
        Ok(WorkspaceMemberWithUser {
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            joined_at: DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    workspace_id: String,
    github_app_installation_id: String,
    repo_owner: String,
    repo_name: String,
    default_branch: String,
    created_at: String,
}

impl TryFrom<ProjectRow> for Project {
    type Error = anyhow::Error;

    fn try_from(row: ProjectRow) -> anyhow::Result<Self> {
        Ok(Project {
            id: row.id,
            workspace_id: row.workspace_id,
            github_app_installation_id: row.github_app_installation_id,
            repo_owner: row.repo_owner,
            repo_name: row.repo_name,
            default_branch: row.default_branch,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

/// Atomically insert a workspace and its creator-as-member row. Either
/// both rows land or neither — prevents a failed `workspace_members`
/// insert from leaving an orphan workspace that permanently consumes
/// its slug.
pub async fn insert_workspace_with_creator(
    pool: &PgPool,
    workspace: &Workspace,
    member: &WorkspaceMember,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&workspace.id)
    .bind(&workspace.slug)
    .bind(&workspace.name)
    .bind(&workspace.created_by)
    .bind(workspace.created_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, joined_at) VALUES ($1, $2, $3)",
    )
    .bind(&member.workspace_id)
    .bind(&member.user_id)
    .bind(member.joined_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_workspace(pool: &PgPool, workspace_id: &str) -> anyhow::Result<Option<Workspace>> {
    let row = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT id, slug, name, created_by, created_at FROM workspaces WHERE id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_workspaces_for_user(
    pool: &PgPool,
    user_id: &str,
) -> anyhow::Result<Vec<Workspace>> {
    let rows = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT w.id, w.slug, w.name, w.created_by, w.created_at \
         FROM workspaces w \
         JOIN workspace_members m ON w.id = m.workspace_id \
         WHERE m.user_id = $1 \
         ORDER BY w.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

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

pub async fn list_workspace_members_with_users(
    pool: &PgPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<WorkspaceMemberWithUser>> {
    let rows = sqlx::query_as::<_, WorkspaceMemberWithUserRow>(
        "SELECT m.workspace_id, m.user_id, m.joined_at, \
                u.github_login, u.github_name, u.github_avatar_url \
         FROM workspace_members m \
         LEFT JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = $1 \
         ORDER BY m.joined_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Read shape the workspace route handlers need to validate that an
/// installation belongs to a workspace and to mint an installation
/// token. Slice 3b moves the `github_app_installations` table into
/// portal's migrations directory; until then portal reads the same
/// table stiglab's runtime migrations created.
pub struct InstallationLookup {
    pub workspace_id: String,
    pub install_id: i64,
}

pub async fn get_installation_lookup(
    pool: &PgPool,
    install_row_id: &str,
) -> anyhow::Result<Option<InstallationLookup>> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT workspace_id, install_id FROM github_app_installations WHERE id = $1",
    )
    .bind(install_row_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(w, i)| InstallationLookup {
        workspace_id: w,
        install_id: i,
    }))
}

pub async fn insert_project(pool: &PgPool, project: &Project) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO projects (id, workspace_id, github_app_installation_id, repo_owner, \
                               repo_name, default_branch, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&project.id)
    .bind(&project.workspace_id)
    .bind(&project.github_app_installation_id)
    .bind(&project.repo_owner)
    .bind(&project.repo_name)
    .bind(&project.default_branch)
    .bind(project.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_project(pool: &PgPool, project_id: &str) -> anyhow::Result<Option<Project>> {
    let row = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_projects_for_workspace(
    pool: &PgPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn list_projects_for_user(pool: &PgPool, user_id: &str) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT p.id, p.workspace_id, p.github_app_installation_id, p.repo_owner, p.repo_name, \
                p.default_branch, p.created_at \
         FROM projects p \
         JOIN workspace_members m ON p.workspace_id = m.workspace_id \
         WHERE m.user_id = $1 \
         ORDER BY p.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn delete_project(pool: &PgPool, project_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count sessions attached to a project that are not in a terminal
/// state. Used to block project deletion while live sessions reference
/// it (no cascade, no soft-delete in v1). Reads stiglab's `sessions`
/// table from the same Postgres instance — the table is stiglab-owned
/// but the column is shared.
pub async fn count_live_sessions_for_project(
    pool: &PgPool,
    project_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions \
         WHERE project_id = $1 AND state NOT IN ('done', 'failed')",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
