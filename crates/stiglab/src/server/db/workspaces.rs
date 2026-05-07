use crate::core::{Workspace, WorkspaceMember};
use chrono::Utc;
use sqlx::AnyPool;

// ── Workspace / membership / installation / project CRUD (issue #59;
//    renamed in #163).

pub async fn insert_workspace(pool: &AnyPool, workspace: &Workspace) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workspaces (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&workspace.id)
    .bind(&workspace.slug)
    .bind(&workspace.name)
    .bind(&workspace.created_by)
    .bind(workspace.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Atomically insert a workspace and its creator-as-member row.  Either
/// both rows land or neither does — prevents a failed `workspace_members`
/// insert from leaving an orphan workspace that permanently consumes its
/// slug.
pub async fn insert_workspace_with_creator(
    pool: &AnyPool,
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

pub async fn get_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Option<Workspace>> {
    let row = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT id, slug, name, created_by, created_at FROM workspaces WHERE id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_workspace_by_slug(
    pool: &AnyPool,
    slug: &str,
) -> anyhow::Result<Option<Workspace>> {
    let row = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT id, slug, name, created_by, created_at FROM workspaces WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_workspaces_for_user(
    pool: &AnyPool,
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

pub async fn insert_workspace_member(
    pool: &AnyPool,
    member: &WorkspaceMember,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, joined_at) VALUES ($1, $2, $3)",
    )
    .bind(&member.workspace_id)
    .bind(&member.user_id)
    .bind(member.joined_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn is_workspace_member(
    pool: &AnyPool,
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

pub async fn list_workspace_members(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<WorkspaceMember>> {
    let rows = sqlx::query_as::<_, WorkspaceMemberRow>(
        "SELECT workspace_id, user_id, joined_at \
         FROM workspace_members WHERE workspace_id = $1 ORDER BY joined_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// `WorkspaceMember` enriched with the member's GitHub profile so the
/// dashboard can render `@login` + avatar instead of the opaque user UUID.
/// `LEFT JOIN` so a member row whose `users` row was somehow removed still
/// surfaces (with nullable GitHub fields) rather than silently disappearing
/// from the workspace's member list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceMemberWithUser {
    pub workspace_id: String,
    pub user_id: String,
    pub joined_at: chrono::DateTime<Utc>,
    pub github_login: Option<String>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
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
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
        })
    }
}

pub async fn list_workspace_members_with_users(
    pool: &AnyPool,
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

// ── Row types ──

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
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceMemberRow {
    workspace_id: String,
    user_id: String,
    joined_at: String,
}

impl TryFrom<WorkspaceMemberRow> for WorkspaceMember {
    type Error = anyhow::Error;

    fn try_from(row: WorkspaceMemberRow) -> anyhow::Result<Self> {
        Ok(WorkspaceMember {
            workspace_id: row.workspace_id,
            user_id: row.user_id,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
        })
    }
}
