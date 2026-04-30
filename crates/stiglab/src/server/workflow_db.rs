//! Workflow + workflow_stages CRUD helpers (issue #81).
//!
//! Kept out of `db.rs` to keep the per-subsystem tables grouped together;
//! the workflow surface is large enough (CRUD + stage chain + active-repo
//! lookup for the webhook router) to justify its own module.

use chrono::{DateTime, Utc};
use sqlx::AnyPool;

use crate::core::workflow::{GateKind, TriggerKind, Workflow, WorkflowStage};

/// Row shape straight out of the `workspace_workflows` table. `i32` is the
/// AnyPool-portable boolean — both SQLite and Postgres store the column as
/// `INTEGER`.
#[derive(sqlx::FromRow)]
struct WorkflowRow {
    id: String,
    workspace_id: String,
    name: String,
    trigger_kind: String,
    repo_owner: String,
    repo_name: String,
    trigger_label: String,
    install_id: i64,
    preset_id: Option<String>,
    active: i32,
    created_by: String,
    created_at: String,
    updated_at: String,
}

impl TryFrom<WorkflowRow> for Workflow {
    type Error = anyhow::Error;
    fn try_from(r: WorkflowRow) -> anyhow::Result<Self> {
        Ok(Workflow {
            id: r.id,
            workspace_id: r.workspace_id,
            name: r.name,
            trigger_kind: r.trigger_kind.parse::<TriggerKind>()?,
            repo_owner: r.repo_owner,
            repo_name: r.repo_name,
            trigger_label: r.trigger_label,
            install_id: r.install_id,
            preset_id: r.preset_id,
            active: r.active != 0,
            created_by: r.created_by,
            created_at: DateTime::parse_from_rfc3339(&r.created_at)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&r.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkflowStageRow {
    id: String,
    workflow_id: String,
    seq: i32,
    gate_kind: String,
    params: String,
}

impl TryFrom<WorkflowStageRow> for WorkflowStage {
    type Error = anyhow::Error;
    fn try_from(r: WorkflowStageRow) -> anyhow::Result<Self> {
        Ok(WorkflowStage {
            id: r.id,
            workflow_id: r.workflow_id,
            seq: r.seq,
            gate_kind: r.gate_kind.parse::<GateKind>()?,
            params: serde_json::from_str(&r.params)?,
        })
    }
}

/// Insert a workflow row plus its ordered stage chain in a single
/// transaction. Rolls back on any error so a partial workflow (header
/// without stages) can't leak into the DB.
pub async fn insert_workflow_with_stages(
    pool: &AnyPool,
    workflow: &Workflow,
    stages: &[WorkflowStage],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO workspace_workflows (id, workspace_id, name, trigger_kind, repo_owner, repo_name, \
                                          trigger_label, install_id, preset_id, active, created_by, \
                                          created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&workflow.id)
    .bind(&workflow.workspace_id)
    .bind(&workflow.name)
    .bind(workflow.trigger_kind.to_string())
    .bind(&workflow.repo_owner)
    .bind(&workflow.repo_name)
    .bind(&workflow.trigger_label)
    .bind(workflow.install_id)
    .bind(workflow.preset_id.as_deref())
    .bind(if workflow.active { 1 } else { 0 })
    .bind(&workflow.created_by)
    .bind(workflow.created_at.to_rfc3339())
    .bind(workflow.updated_at.to_rfc3339())
    .execute(&mut *tx)
    .await?;

    for s in stages {
        sqlx::query(
            "INSERT INTO workspace_workflow_stages (id, workflow_id, seq, gate_kind, params) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&s.id)
        .bind(&s.workflow_id)
        .bind(s.seq)
        .bind(s.gate_kind.to_string())
        .bind(serde_json::to_string(&s.params)?)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_workflow(pool: &AnyPool, workflow_id: &str) -> anyhow::Result<Option<Workflow>> {
    let row = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows WHERE id = $1",
    )
    .bind(workflow_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_workflows_for_workspace(
    pool: &AnyPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let rows = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// List every workflow across workspaces. Used by the spine-mirror startup
/// backfill so workflows pre-dating the bridge sync into the spine schema.
pub async fn list_all_workflows(pool: &AnyPool) -> anyhow::Result<Vec<Workflow>> {
    let rows = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn list_stages_for_workflow(
    pool: &AnyPool,
    workflow_id: &str,
) -> anyhow::Result<Vec<WorkflowStage>> {
    let rows = sqlx::query_as::<_, WorkflowStageRow>(
        "SELECT id, workflow_id, seq, gate_kind, params FROM workspace_workflow_stages \
         WHERE workflow_id = $1 ORDER BY seq ASC",
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Toggle the `active` flag and bump `updated_at`.
pub async fn set_workflow_active(
    pool: &AnyPool,
    workflow_id: &str,
    active: bool,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE workspace_workflows SET active = $1, updated_at = $2 WHERE id = $3")
        .bind(if active { 1 } else { 0 })
        .bind(Utc::now().to_rfc3339())
        .bind(workflow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete the workflow row and its ordered stage chain in a single
/// transaction. Callers are expected to have deactivated the workflow
/// first (the label-watcher side effect is undone before the row goes
/// away); this helper only touches the database state.
pub async fn delete_workflow(pool: &AnyPool, workflow_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM workspace_workflow_stages WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM workspace_workflows WHERE id = $1")
        .bind(workflow_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Find every **active** workflow whose `github-issue-webhook` trigger
/// targets this repo and matches the supplied label set. The webhook router
/// calls this to decide which workflows should fire `trigger.fired` for a
/// given `issues.labeled` payload.
pub async fn find_active_github_workflows_for_label(
    pool: &AnyPool,
    repo_owner: &str,
    repo_name: &str,
    label: &str,
) -> anyhow::Result<Vec<Workflow>> {
    // Bind the trigger-kind string via the enum's `Display` so the SQL
    // doesn't drift if the string representation ever changes.
    let trigger_kind = TriggerKind::GithubIssueWebhook.to_string();
    let rows = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows \
         WHERE active = 1 AND trigger_kind = $1 \
           AND repo_owner = $2 AND repo_name = $3 AND trigger_label = $4",
    )
    .bind(&trigger_kind)
    .bind(repo_owner)
    .bind(repo_name)
    .bind(label)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Find every **active** workflow whose `github-issue-webhook` trigger
/// targets this repo + label, scoped to a single workspace. Used by the
/// dashboard's manual-replay route so the matched set respects the
/// caller's workspace boundary even when the same `(repo_owner, repo_name)`
/// is connected to multiple workspaces.
pub async fn find_active_github_workflows_for_label_in_workspace(
    pool: &AnyPool,
    workspace_id: &str,
    repo_owner: &str,
    repo_name: &str,
    label: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let trigger_kind = TriggerKind::GithubIssueWebhook.to_string();
    let rows = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows \
         WHERE active = 1 AND workspace_id = $1 AND trigger_kind = $2 \
           AND repo_owner = $3 AND repo_name = $4 AND trigger_label = $5",
    )
    .bind(workspace_id)
    .bind(&trigger_kind)
    .bind(repo_owner)
    .bind(repo_name)
    .bind(label)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// All active `github-issue-webhook` workflows for `(workspace, repo)` —
/// returned without label filtering so callers iterating over many
/// candidate labels can do one round-trip and partition in-memory.
pub async fn find_active_github_workflows_for_workspace_repo(
    pool: &AnyPool,
    workspace_id: &str,
    repo_owner: &str,
    repo_name: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let trigger_kind = TriggerKind::GithubIssueWebhook.to_string();
    let rows = sqlx::query_as::<_, WorkflowRow>(
        "SELECT id, workspace_id, name, trigger_kind, repo_owner, repo_name, trigger_label, \
                install_id, preset_id, active, created_by, created_at, updated_at \
         FROM workspace_workflows \
         WHERE active = 1 AND workspace_id = $1 AND trigger_kind = $2 \
           AND repo_owner = $3 AND repo_name = $4",
    )
    .bind(workspace_id)
    .bind(&trigger_kind)
    .bind(repo_owner)
    .bind(repo_name)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

/// Whether any other active workflow on `(repo_owner, repo_name)` still
/// needs webhook delivery. Used by the deactivation hook to decide if it
/// can deregister the repo-level webhook.
pub async fn any_other_active_workflow_on_repo(
    pool: &AnyPool,
    repo_owner: &str,
    repo_name: &str,
    exclude_workflow_id: &str,
) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workspace_workflows \
         WHERE active = 1 AND repo_owner = $1 AND repo_name = $2 AND id <> $3",
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(exclude_workflow_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
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

/// Resolve `(install_id, repo_owner, repo_name)` for a workflow — used by the
/// activation hook to know which install token to mint.
pub async fn get_workflow_install_target(
    pool: &AnyPool,
    workflow_id: &str,
) -> anyhow::Result<Option<(i64, String, String)>> {
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT install_id, repo_owner, repo_name FROM workspace_workflows WHERE id = $1",
    )
    .bind(workflow_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workflow::{GateKind, TriggerKind};
    use crate::server::db::run_migrations;
    use chrono::Utc;
    use serde_json::json;
    use sqlx::pool::PoolOptions;
    use uuid::Uuid;

    async fn pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = PoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite in-memory connect");
        run_migrations(&pool).await.expect("migrations");
        pool
    }

    fn sample_workflow(workspace: &str, repo: &str, label: &str, active: bool) -> Workflow {
        let now = Utc::now();
        Workflow {
            id: format!("wf_{}", Uuid::new_v4()),
            workspace_id: workspace.to_string(),
            name: "sdd".to_string(),
            trigger_kind: TriggerKind::GithubIssueWebhook,
            repo_owner: "acme".to_string(),
            repo_name: repo.to_string(),
            trigger_label: label.to_string(),
            install_id: 42,
            preset_id: Some("github-issue-to-pr".to_string()),
            active,
            created_by: "u1".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn agent_stage(workflow_id: &str) -> WorkflowStage {
        WorkflowStage {
            id: Uuid::new_v4().to_string(),
            workflow_id: workflow_id.to_string(),
            seq: 0,
            gate_kind: GateKind::AgentSession,
            params: json!({"action": "implement-and-open-pr"}),
        }
    }

    #[tokio::test]
    async fn insert_and_get_round_trip() {
        let pool = pool().await;
        let wf = sample_workflow("w1", "widgets", "spec", false);
        let stage = agent_stage(&wf.id);
        insert_workflow_with_stages(&pool, &wf, std::slice::from_ref(&stage))
            .await
            .unwrap();

        let loaded = get_workflow(&pool, &wf.id).await.unwrap().unwrap();
        assert_eq!(loaded.id, wf.id);
        assert_eq!(loaded.trigger_kind, TriggerKind::GithubIssueWebhook);
        assert!(!loaded.active);

        let stages = list_stages_for_workflow(&pool, &wf.id).await.unwrap();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].gate_kind, GateKind::AgentSession);
    }

    #[tokio::test]
    async fn active_lookup_filters_on_label_and_repo() {
        let pool = pool().await;
        let wf_a = sample_workflow("w1", "widgets", "spec", true);
        let wf_b = sample_workflow("w1", "widgets", "bug", true);
        let wf_c = sample_workflow("w1", "widgets", "spec", false); // inactive
        let wf_d = sample_workflow("w1", "gadgets", "spec", true); // wrong repo
        for wf in [&wf_a, &wf_b, &wf_c, &wf_d] {
            insert_workflow_with_stages(&pool, wf, std::slice::from_ref(&agent_stage(&wf.id)))
                .await
                .unwrap();
        }

        let hits = find_active_github_workflows_for_label(&pool, "acme", "widgets", "spec")
            .await
            .unwrap();
        let ids: Vec<_> = hits.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec![wf_a.id.as_str()]);
    }

    #[tokio::test]
    async fn workspace_scoped_lookup_excludes_other_workspaces() {
        let pool = pool().await;
        let wf_a = sample_workflow("w1", "widgets", "spec", true);
        let wf_b = sample_workflow("w2", "widgets", "spec", true);
        for wf in [&wf_a, &wf_b] {
            insert_workflow_with_stages(&pool, wf, std::slice::from_ref(&agent_stage(&wf.id)))
                .await
                .unwrap();
        }
        let hits = find_active_github_workflows_for_label_in_workspace(
            &pool, "w1", "acme", "widgets", "spec",
        )
        .await
        .unwrap();
        let ids: Vec<_> = hits.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec![wf_a.id.as_str()]);
    }

    #[tokio::test]
    async fn set_active_and_any_other_active_works() {
        let pool = pool().await;
        let wf_a = sample_workflow("w1", "widgets", "spec", true);
        let wf_b = sample_workflow("w1", "widgets", "bug", true);
        for wf in [&wf_a, &wf_b] {
            insert_workflow_with_stages(&pool, wf, std::slice::from_ref(&agent_stage(&wf.id)))
                .await
                .unwrap();
        }
        assert!(
            any_other_active_workflow_on_repo(&pool, "acme", "widgets", &wf_a.id)
                .await
                .unwrap()
        );
        set_workflow_active(&pool, &wf_b.id, false).await.unwrap();
        assert!(
            !any_other_active_workflow_on_repo(&pool, "acme", "widgets", &wf_a.id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn delete_workflow_drops_row_and_stages() {
        let pool = pool().await;
        let wf = sample_workflow("w1", "widgets", "spec", false);
        insert_workflow_with_stages(&pool, &wf, std::slice::from_ref(&agent_stage(&wf.id)))
            .await
            .unwrap();

        delete_workflow(&pool, &wf.id).await.unwrap();

        assert!(get_workflow(&pool, &wf.id).await.unwrap().is_none());
        let stages = list_stages_for_workflow(&pool, &wf.id).await.unwrap();
        assert!(stages.is_empty());
    }

    #[tokio::test]
    async fn list_workflows_for_workspace_scopes() {
        let pool = pool().await;
        let wf_a = sample_workflow("w1", "widgets", "spec", true);
        let wf_b = sample_workflow("w2", "widgets", "spec", true);
        for wf in [&wf_a, &wf_b] {
            insert_workflow_with_stages(&pool, wf, std::slice::from_ref(&agent_stage(&wf.id)))
                .await
                .unwrap();
        }
        let w1 = list_workflows_for_workspace(&pool, "w1").await.unwrap();
        assert_eq!(w1.len(), 1);
        assert_eq!(w1[0].workspace_id, "w1");
    }
}
