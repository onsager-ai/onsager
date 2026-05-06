//! Read-only workflow lookups against the spine `workflows` /
//! `workflow_stages` tables.
//!
//! Spec #222 Slice 4 moved the writer surface (insert / update / delete
//! / activation) and the GitHub side-effects to portal. Stiglab still
//! reads these rows for the in-process needs of `routes/projects.rs`
//! replay-trigger — same database, separate connection pool, portal is
//! the only writer.
//!
//! Pool: spine `PgPool`. The spine schema is Postgres-only and `JSONB`
//! / `BOOLEAN` / `TIMESTAMPTZ` round-trips don't go through `Any`.

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::core::workflow::{TriggerKind, Workflow};

/// All active `github-issue-webhook` workflows for `(workspace, repo)` —
/// returned without label filtering so callers iterating over many
/// candidate labels can do one round-trip and partition in-memory.
pub async fn find_active_github_workflows_for_workspace_repo(
    pool: &PgPool,
    workspace_id: &str,
    repo_owner: &str,
    repo_name: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let repo = format!("{repo_owner}/{repo_name}");
    let rows = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_id, install_id, created_by, created_at, updated_at \
           FROM workflows \
          WHERE active = TRUE \
            AND workspace_id = $1 \
            AND trigger_kind = 'github_issue_webhook' \
            AND trigger_config ->> 'repo' = $2",
    )
    .bind(workspace_id)
    .bind(&repo)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_workflow).collect()
}

fn row_to_workflow(row: sqlx::postgres::PgRow) -> anyhow::Result<Workflow> {
    let id: String = row.try_get("workflow_id")?;
    let name: String = row.try_get("name")?;
    let trigger_kind_raw: String = row.try_get("trigger_kind")?;
    let trigger_config: serde_json::Value = row.try_get("trigger_config")?;
    let active: bool = row.try_get("active")?;
    let preset_id: Option<String> = row.try_get("preset_id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let install_id_text: Option<String> = row.try_get("install_id")?;
    let created_by: Option<String> = row.try_get("created_by")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    let trigger = TriggerKind::from_storage(&trigger_kind_raw, &trigger_config)
        .with_context(|| format!("workflow {id} has unparseable trigger"))?;
    let install_id: i64 = install_id_text
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow::anyhow!("workflow {id} has missing/non-numeric install_id"))?;

    Ok(Workflow {
        id,
        workspace_id,
        name,
        trigger,
        install_id,
        preset_id,
        active,
        created_by: created_by.unwrap_or_default(),
        created_at,
        updated_at,
    })
}
