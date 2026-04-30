// seam-allow: removed in Lever D (#149) — collapses workspace_workflows into spine workflows with a workspace_id discriminator
//! Mirror stiglab's `workspace_workflows` rows into the spine `workflows` /
//! `workflow_stages` tables that forge reads.
//!
//! Stiglab owns the multi-workspace CRUD schema (`workspace_workflows`);
//! forge consumes the spine schema defined in `crates/onsager-spine/
//! migrations/006_workflows.sql`. Without this bridge, every
//! `trigger.fired` event emitted by stiglab references a `workflow_id`
//! forge can't resolve, and the trigger is dropped with a "trigger.fired
//! for unknown workflow" warning (no artifact, no session).
//!
//! Called from every CRUD handler that mutates a workflow, plus a one-shot
//! backfill on startup so workflows that pre-date this bridge sync over.

use anyhow::Context;
use serde_json::json;
use sqlx::PgPool;

use crate::core::workflow::{GateKind, TriggerKind, Workflow, WorkflowStage};

/// Insert or update the spine workflow row + its full stage chain. Stages
/// are replaced wholesale on every call (delete-then-insert) so removing a
/// stage in stiglab actually removes it from the spine.
pub async fn upsert(
    spine_pool: &PgPool,
    workflow: &Workflow,
    stages: &[WorkflowStage],
) -> anyhow::Result<()> {
    let mut tx = spine_pool.begin().await?;

    let trigger_kind = trigger_kind_to_spine(workflow.trigger_kind);
    let trigger_config = trigger_config_for(workflow);
    let install_id_text = workflow.install_id.to_string();

    // `created_by` (issue #156) is the owner identity stiglab uses to
    // decrypt CLAUDE_CODE_OAUTH_TOKEN at shaping-dispatch time. Mirror it
    // through to the spine so forge can read it from the same `workflows`
    // table without a stiglab DB roundtrip. ON CONFLICT updates it too:
    // re-activating a legacy workflow attaches the activator's id and
    // unblocks the dispatch path the next time the gate evaluates.
    sqlx::query(
        "INSERT INTO workflows (workflow_id, name, trigger_kind, trigger_config, \
                                active, preset_id, install_id, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (workflow_id) DO UPDATE SET \
             name = EXCLUDED.name, \
             trigger_kind = EXCLUDED.trigger_kind, \
             trigger_config = EXCLUDED.trigger_config, \
             active = EXCLUDED.active, \
             preset_id = EXCLUDED.preset_id, \
             install_id = EXCLUDED.install_id, \
             created_by = EXCLUDED.created_by",
    )
    .bind(&workflow.id)
    .bind(&workflow.name)
    .bind(trigger_kind)
    .bind(&trigger_config)
    .bind(workflow.active)
    .bind(workflow.preset_id.as_deref())
    .bind(&install_id_text)
    .bind(&workflow.created_by)
    .execute(&mut *tx)
    .await
    .context("upsert spine workflows row")?;

    sqlx::query("DELETE FROM workflow_stages WHERE workflow_id = $1")
        .bind(&workflow.id)
        .execute(&mut *tx)
        .await
        .context("clear spine workflow_stages")?;

    for stage in stages {
        let (target_state, gates) = translate_stage(stage.gate_kind, &stage.params);
        sqlx::query(
            "INSERT INTO workflow_stages (workflow_id, stage_order, name, target_state, \
                                          gates, params) \
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

/// Remove the spine workflow row + cascading stages. Called from
/// stiglab's delete handler so forge stops resolving a workflow that no
/// longer exists in stiglab.
pub async fn delete(spine_pool: &PgPool, workflow_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM workflows WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(spine_pool)
        .await
        .context("delete spine workflows row")?;
    Ok(())
}

/// One-shot startup sync. Reads every `workspace_workflows` row plus its
/// stages and upserts the spine schema. Idempotent; safe to run on every
/// boot. Logs a warning per failed row but does not abort startup —
/// individual translation failures shouldn't take stiglab down.
pub async fn backfill(stiglab_pool: &sqlx::AnyPool, spine_pool: &PgPool) -> anyhow::Result<usize> {
    let workflows = crate::server::workflow_db::list_all_workflows(stiglab_pool).await?;
    let mut synced = 0usize;
    for w in workflows {
        let stages =
            match crate::server::workflow_db::list_stages_for_workflow(stiglab_pool, &w.id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(workflow_id = %w.id, "backfill: stage load failed: {e}");
                    continue;
                }
            };
        if let Err(e) = upsert(spine_pool, &w, &stages).await {
            tracing::warn!(workflow_id = %w.id, "backfill: spine upsert failed: {e}");
            continue;
        }
        synced += 1;
    }
    Ok(synced)
}

fn trigger_kind_to_spine(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::GithubIssueWebhook => "github_issue_webhook",
    }
}

fn trigger_config_for(workflow: &Workflow) -> serde_json::Value {
    match workflow.trigger_kind {
        TriggerKind::GithubIssueWebhook => json!({
            "repo": format!("{}/{}", workflow.repo_owner, workflow.repo_name),
            "label": workflow.trigger_label,
        }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn trigger_config_pairs_repo_and_label() {
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
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let cfg = trigger_config_for(&w);
        assert_eq!(cfg["repo"], "owner/repo");
        assert_eq!(cfg["label"], "planned");
    }
}
