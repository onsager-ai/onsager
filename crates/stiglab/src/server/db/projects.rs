use crate::core::Project;
use chrono::Utc;
use sqlx::AnyPool;

pub async fn insert_project(pool: &AnyPool, project: &Project) -> anyhow::Result<()> {
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

pub async fn get_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<Option<Project>> {
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
    pool: &AnyPool,
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

pub async fn list_projects_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Project>> {
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

pub async fn delete_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count sessions attached to a project that are not in a terminal state.
/// Used to block project deletion while live sessions reference it (no
/// cascade, no soft-delete in v1).
pub async fn count_live_sessions_for_project(
    pool: &AnyPool,
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

// ── Row types ──

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
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}
