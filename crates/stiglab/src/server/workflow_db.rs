//! Workflow CRUD against the spine `workflows` / `workflow_stages` tables.
//!
//! Lever D (#149): stiglab no longer keeps a private `workspace_workflows`
//! schema mirrored into the spine — there is one table, owned by the
//! spine, and stiglab writes to it directly. The translation that used to
//! live in `workflow_spine_mirror.rs` (gate_kind → target_state + gates
//! JSON, repo + label → trigger_config JSON) is folded into this module,
//! where it is the single read/write boundary.
//!
//! Pool: spine `PgPool`. The earlier `AnyPool` against stiglab's own
//! database is gone; the spine schema is Postgres-only and `JSONB` /
//! `BOOLEAN` / `TIMESTAMPTZ` round-trips don't go through `Any`.
//!
//! In production stiglab and spine point at the same Postgres, so this is
//! just a pool name change. In topologies where they're separate, workflow
//! routes 503 when the spine is unreachable — same posture as the rest of
//! the workflow surface (`list_workflow_runs` already returns an empty
//! list without a spine).

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::core::workflow::{GateKind, TriggerKind, Workflow, WorkflowStage};

// ── Translation between stiglab's API types and the spine schema ──────────

/// stiglab persists `'github-issue-webhook'` (kebab); the spine
/// `workflows.trigger_kind` CHECK constraint requires the snake form.
fn trigger_kind_to_spine(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::GithubIssueWebhook => "github_issue_webhook",
    }
}

fn trigger_kind_from_spine(s: &str) -> anyhow::Result<TriggerKind> {
    match s {
        "github_issue_webhook" => Ok(TriggerKind::GithubIssueWebhook),
        other => Err(anyhow::anyhow!("unknown spine trigger_kind: {other}")),
    }
}

/// Pack the per-row GitHub trigger fields into the JSON shape forge reads.
fn trigger_config_for(workflow: &Workflow) -> serde_json::Value {
    match workflow.trigger_kind {
        TriggerKind::GithubIssueWebhook => json!({
            "repo": format!("{}/{}", workflow.repo_owner, workflow.repo_name),
            "label": workflow.trigger_label,
        }),
    }
}

/// Reverse of [`trigger_config_for`]. The mirror has been writing the
/// `{repo, label}` shape since it shipped, so every spine row that exists
/// today round-trips cleanly. Returns `(repo_owner, repo_name, label)`.
fn parse_trigger_config(cfg: &serde_json::Value) -> anyhow::Result<(String, String, String)> {
    let repo = cfg
        .get("repo")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("trigger_config.repo missing or non-string"))?;
    let label = cfg
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("trigger_config.label missing or non-string"))?;
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("trigger_config.repo is not 'owner/name': {repo}"))?;
    Ok((owner.to_string(), name.to_string(), label.to_string()))
}

/// Translate stiglab's `gate_kind` + opaque `params` into the spine's
/// `(target_state, gates)` pair. The artifact-state transitions match the
/// "issue → PR" flow forge expects: agent-session moves Draft → InProgress,
/// review-style gates move to UnderReview.
fn translate_stage(
    gate_kind: GateKind,
    params: &serde_json::Value,
) -> (Option<&'static str>, serde_json::Value) {
    match gate_kind {
        GateKind::AgentSession => {
            let gate = json!({
                "kind": "agent_session",
                "shaping_intent": params.clone(),
            });
            (Some("in_progress"), json!([gate]))
        }
        GateKind::ExternalCheck => {
            let check_name = params
                .get("check_name")
                .and_then(|v| v.as_str())
                .unwrap_or("ci");
            let gate = json!({
                "kind": "external_check",
                "check_name": check_name,
            });
            (Some("under_review"), json!([gate]))
        }
        GateKind::Governance => {
            let gate_point = params.get("gate_point").and_then(|v| v.as_str());
            let gate = match gate_point {
                Some(p) => json!({"kind": "governance", "gate_point": p}),
                None => json!({"kind": "governance"}),
            };
            (Some("under_review"), json!([gate]))
        }
        GateKind::ManualApproval => {
            let signal_kind = params
                .get("signal_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("dashboard_approve");
            let gate = json!({
                "kind": "manual_approval",
                "signal_kind": signal_kind,
            });
            (Some("under_review"), json!([gate]))
        }
    }
}

// ── Public CRUD surface ───────────────────────────────────────────────────

/// Insert a workflow row plus its ordered stage chain in a single
/// transaction. Both ends of the spine `workflows` ↔ `workflow_stages`
/// FK move atomically so a partial workflow can't leak.
pub async fn insert_workflow_with_stages(
    pool: &PgPool,
    workflow: &Workflow,
    stages: &[WorkflowStage],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    let trigger_kind = trigger_kind_to_spine(workflow.trigger_kind);
    let trigger_config = trigger_config_for(workflow);
    let install_id_text = workflow.install_id.to_string();

    sqlx::query(
        "INSERT INTO workflows (workflow_id, name, trigger_kind, trigger_config, \
                                active, preset_id, workspace_id, install_id, \
                                created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&workflow.id)
    .bind(&workflow.name)
    .bind(trigger_kind)
    .bind(&trigger_config)
    .bind(workflow.active)
    .bind(workflow.preset_id.as_deref())
    .bind(&workflow.workspace_id)
    .bind(&install_id_text)
    .bind(&workflow.created_by)
    .bind(workflow.created_at)
    .bind(workflow.updated_at)
    .execute(&mut *tx)
    .await
    .context("insert spine workflows row")?;

    for stage in stages {
        let (target_state, gates) = translate_stage(stage.gate_kind, &stage.params);
        sqlx::query(
            "INSERT INTO workflow_stages (workflow_id, stage_order, name, \
                                          target_state, gates, params) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&workflow.id)
        .bind(stage.seq)
        .bind(stage.gate_kind.to_string())
        .bind(target_state)
        .bind(&gates)
        .bind(&stage.params)
        .execute(&mut *tx)
        .await
        .context("insert spine workflow_stages row")?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_workflow(pool: &PgPool, workflow_id: &str) -> anyhow::Result<Option<Workflow>> {
    let row = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_id, install_id, created_by, created_at, updated_at \
           FROM workflows WHERE workflow_id = $1",
    )
    .bind(workflow_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_workflow).transpose()
}

pub async fn list_workflows_for_workspace(
    pool: &PgPool,
    workspace_id: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let rows = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_id, install_id, created_by, created_at, updated_at \
           FROM workflows WHERE workspace_id = $1 ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_workflow).collect()
}

pub async fn list_stages_for_workflow(
    pool: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Vec<WorkflowStage>> {
    let rows = sqlx::query(
        "SELECT workflow_id, stage_order, name, params \
           FROM workflow_stages WHERE workflow_id = $1 ORDER BY stage_order ASC",
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_stage).collect()
}

/// Toggle the `active` flag and bump `updated_at`. The `workflow_updated`
/// trigger on the spine table sets `updated_at = NOW()` automatically; we
/// don't need a manual bind.
pub async fn set_workflow_active(
    pool: &PgPool,
    workflow_id: &str,
    active: bool,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE workflows SET active = $1 WHERE workflow_id = $2")
        .bind(active)
        .bind(workflow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete the workflow row. `workflow_stages` is `ON DELETE CASCADE` so
/// the stage chain goes with it — no explicit per-stage DELETE.
pub async fn delete_workflow(pool: &PgPool, workflow_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM workflows WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Find every **active** workflow whose `github-issue-webhook` trigger
/// targets this repo and matches the supplied label. The webhook router
/// calls this to decide which workflows should fire `trigger.fired` for a
/// given `issues.labeled` payload.
///
/// The match runs against `trigger_config -> 'repo'` and
/// `trigger_config -> 'label'` because spine packs those into the JSON
/// (mirror history); GIN-indexing isn't worth it at v1's volume.
pub async fn find_active_github_workflows_for_label(
    pool: &PgPool,
    install_id: i64,
    repo_owner: &str,
    repo_name: &str,
    label: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let repo = format!("{repo_owner}/{repo_name}");
    let install_id_text = install_id.to_string();
    // Filter on `install_id` too: GitHub deliveries carry the install
    // they came from, and two workspaces can each watch the same
    // `(repo, label)` through different installs. Without this clause
    // a webhook delivered for install A would also fire workflows
    // registered under install B.
    let rows = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_id, install_id, created_by, created_at, updated_at \
           FROM workflows \
          WHERE active = TRUE \
            AND trigger_kind = 'github_issue_webhook' \
            AND install_id = $1 \
            AND trigger_config ->> 'repo'  = $2 \
            AND trigger_config ->> 'label' = $3",
    )
    .bind(&install_id_text)
    .bind(&repo)
    .bind(label)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_workflow).collect()
}

/// Same as [`find_active_github_workflows_for_label`] but scoped to a
/// single workspace. Used by the dashboard's manual-replay route so the
/// matched set respects the caller's workspace boundary even when the
/// same `(repo_owner, repo_name)` is connected to multiple workspaces.
pub async fn find_active_github_workflows_for_label_in_workspace(
    pool: &PgPool,
    workspace_id: &str,
    repo_owner: &str,
    repo_name: &str,
    label: &str,
) -> anyhow::Result<Vec<Workflow>> {
    let repo = format!("{repo_owner}/{repo_name}");
    let rows = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_id, install_id, created_by, created_at, updated_at \
           FROM workflows \
          WHERE active = TRUE \
            AND workspace_id = $1 \
            AND trigger_kind = 'github_issue_webhook' \
            AND trigger_config ->> 'repo'  = $2 \
            AND trigger_config ->> 'label' = $3",
    )
    .bind(workspace_id)
    .bind(&repo)
    .bind(label)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_workflow).collect()
}

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

/// Whether any other active workflow on `(repo_owner, repo_name)` still
/// needs webhook delivery. Used by the deactivation hook to decide if it
/// can deregister the repo-level webhook.
pub async fn any_other_active_workflow_on_repo(
    pool: &PgPool,
    repo_owner: &str,
    repo_name: &str,
    exclude_workflow_id: &str,
) -> anyhow::Result<bool> {
    let repo = format!("{repo_owner}/{repo_name}");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflows \
          WHERE active = TRUE \
            AND trigger_kind = 'github_issue_webhook' \
            AND trigger_config ->> 'repo' = $1 \
            AND workflow_id <> $2",
    )
    .bind(&repo)
    .bind(exclude_workflow_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// Resolve `(install_id, repo_owner, repo_name)` for a workflow — used by
/// the activation hook to know which install token to mint.
pub async fn get_workflow_install_target(
    pool: &PgPool,
    workflow_id: &str,
) -> anyhow::Result<Option<(i64, String, String)>> {
    let row =
        sqlx::query("SELECT install_id, trigger_config FROM workflows WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_optional(pool)
            .await?;
    let Some(row) = row else { return Ok(None) };
    let install_id_text: Option<String> = row.try_get("install_id").ok();
    let install_id: i64 = install_id_text
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| {
            anyhow::anyhow!("workflow {workflow_id} has missing/non-numeric install_id")
        })?;
    let cfg: serde_json::Value = row.try_get("trigger_config")?;
    let (owner, name, _) = parse_trigger_config(&cfg)?;
    Ok(Some((install_id, owner, name)))
}

// ── Row → struct helpers ──────────────────────────────────────────────────

fn row_to_workflow(row: sqlx::postgres::PgRow) -> anyhow::Result<Workflow> {
    let id: String = row.try_get("workflow_id")?;
    let name: String = row.try_get("name")?;
    let trigger_kind_raw: String = row.try_get("trigger_kind")?;
    let trigger_config: serde_json::Value = row.try_get("trigger_config")?;
    let active: bool = row.try_get("active")?;
    let preset_id: Option<String> = row.try_get("preset_id")?;
    let workspace_id: String = row.try_get("workspace_id")?;
    let install_id_text: Option<String> = row.try_get("install_id")?;
    // Typed read so a missing column errors loudly. NULL is still
    // legal — spine migration 009 made `created_by` nullable, and
    // pre-#156 rows persist that way until the owner re-activates.
    // We collapse NULL to an empty string at the API boundary; the
    // activation guard then fails the credential check with
    // `no_credentials_for_workflow`, which is the user-visible signal
    // to re-activate.
    let created_by: Option<String> = row.try_get("created_by")?;
    if created_by.is_none() {
        tracing::warn!(
            workflow_id = %row.try_get::<String, _>("workflow_id").unwrap_or_default(),
            "workflow row has NULL created_by; activation will fail until owner re-activates"
        );
    }
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

    let trigger_kind = trigger_kind_from_spine(&trigger_kind_raw)?;
    let (repo_owner, repo_name, trigger_label) = parse_trigger_config(&trigger_config)?;
    let install_id: i64 = install_id_text
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow::anyhow!("workflow {id} has missing/non-numeric install_id"))?;

    Ok(Workflow {
        id,
        workspace_id,
        name,
        trigger_kind,
        repo_owner,
        repo_name,
        trigger_label,
        install_id,
        preset_id,
        active,
        created_by: created_by.unwrap_or_default(),
        created_at,
        updated_at,
    })
}

/// Stage rows on spine don't carry an explicit `id` (PK is
/// `(workflow_id, stage_order)`). Synthesize a stable `${workflow_id}#${seq}`
/// string for the dashboard's per-stage anchors — the same input always
/// produces the same id, which is the contract callers rely on.
fn row_to_stage(row: sqlx::postgres::PgRow) -> anyhow::Result<WorkflowStage> {
    let workflow_id: String = row.try_get("workflow_id")?;
    let stage_order: i32 = row.try_get("stage_order")?;
    let name: String = row.try_get("name")?;
    let params: serde_json::Value = row.try_get("params")?;
    let gate_kind = name
        .parse::<GateKind>()
        .map_err(|e| anyhow::anyhow!("workflow_stages.name not a known gate kind ({name}): {e}"))?;
    Ok(WorkflowStage {
        id: format!("{workflow_id}#{stage_order}"),
        workflow_id,
        seq: stage_order,
        gate_kind,
        params,
    })
}

#[cfg(test)]
mod tests {
    //! Pure-function unit tests for the translation helpers. Round-trip
    //! coverage against a real Postgres lives in
    //! `tests/workflow_db_pg.rs` (gated on `DATABASE_URL`).

    use super::*;
    use crate::core::workflow::{TriggerKind, Workflow};

    #[test]
    fn agent_session_maps_to_in_progress() {
        let (state, gates) =
            translate_stage(GateKind::AgentSession, &json!({"action": "implement"}));
        assert_eq!(state, Some("in_progress"));
        assert_eq!(gates[0]["kind"], "agent_session");
        assert_eq!(gates[0]["shaping_intent"]["action"], "implement");
    }

    #[test]
    fn external_check_pulls_check_name() {
        let (state, gates) =
            translate_stage(GateKind::ExternalCheck, &json!({"check_name": "ci/test"}));
        assert_eq!(state, Some("under_review"));
        assert_eq!(gates[0]["kind"], "external_check");
        assert_eq!(gates[0]["check_name"], "ci/test");
    }

    #[test]
    fn manual_approval_defaults_signal_kind() {
        let (_, gates) = translate_stage(GateKind::ManualApproval, &json!({}));
        assert_eq!(gates[0]["signal_kind"], "dashboard_approve");
    }

    #[test]
    fn trigger_kind_round_trips_through_spine_form() {
        let s = trigger_kind_to_spine(TriggerKind::GithubIssueWebhook);
        assert_eq!(s, "github_issue_webhook");
        assert_eq!(
            trigger_kind_from_spine(s).unwrap(),
            TriggerKind::GithubIssueWebhook
        );
    }

    #[test]
    fn trigger_config_round_trips() {
        let w = Workflow {
            id: "wf_x".into(),
            workspace_id: "w".into(),
            name: "x".into(),
            trigger_kind: TriggerKind::GithubIssueWebhook,
            repo_owner: "owner".into(),
            repo_name: "repo".into(),
            trigger_label: "planned".into(),
            install_id: 42,
            preset_id: None,
            active: true,
            created_by: "u".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let cfg = trigger_config_for(&w);
        assert_eq!(cfg["repo"], "owner/repo");
        assert_eq!(cfg["label"], "planned");
        let (owner, name, label) = parse_trigger_config(&cfg).unwrap();
        assert_eq!(
            (owner.as_str(), name.as_str(), label.as_str()),
            ("owner", "repo", "planned")
        );
    }

    #[test]
    fn parse_trigger_config_rejects_malformed() {
        assert!(parse_trigger_config(&json!({})).is_err());
        assert!(parse_trigger_config(&json!({"repo": "no-slash", "label": "x"})).is_err());
        assert!(parse_trigger_config(&json!({"repo": "a/b"})).is_err());
    }
}
