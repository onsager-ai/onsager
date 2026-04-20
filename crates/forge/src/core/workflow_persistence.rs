//! Workflow persistence (issue #80).
//!
//! Reads the `workflows` + `workflow_stages` tables at startup and maps
//! them into the [`Workflow`] domain model. Also provides the UPDATE path
//! for an artifact's workflow tagging columns so the stage runner's
//! transitions survive a restart.

use std::collections::HashMap;

use onsager_artifact::{Artifact, ArtifactState};
use sqlx::{PgPool, Row};

use super::persistence::state_to_db;
use super::workflow::{GateSpec, StageSpec, TriggerSpec, Workflow};

/// Load every active workflow into memory. Inactive workflows with
/// in-flight artifacts are also needed so the stage runner can continue
/// walking them to completion.
pub async fn load_workflows(pool: &PgPool) -> Result<HashMap<String, Workflow>, sqlx::Error> {
    let wf_rows = sqlx::query(
        "SELECT workflow_id, name, trigger_kind, trigger_config, active, preset_id, \
                workspace_install_ref \
           FROM workflows",
    )
    .fetch_all(pool)
    .await?;

    let mut workflows = HashMap::new();
    for row in wf_rows {
        let id: String = row.get("workflow_id");
        let name: String = row.get("name");
        let trigger_kind: String = row.get("trigger_kind");
        let trigger_config: serde_json::Value = row.get("trigger_config");
        let active: bool = row.get("active");
        let preset_id: Option<String> = row.get("preset_id");
        let workspace_install_ref: Option<String> = row.get("workspace_install_ref");

        let trigger = match parse_trigger(&trigger_kind, &trigger_config) {
            Some(t) => t,
            None => {
                tracing::warn!(
                    workflow_id = %id,
                    trigger_kind = %trigger_kind,
                    "skipping workflow with unrecognized trigger"
                );
                continue;
            }
        };

        let stage_rows = sqlx::query(
            "SELECT stage_order, name, target_state, gates, params \
               FROM workflow_stages WHERE workflow_id = $1 \
               ORDER BY stage_order ASC",
        )
        .bind(&id)
        .fetch_all(pool)
        .await?;

        let mut stages = Vec::with_capacity(stage_rows.len());
        for srow in stage_rows {
            let stage_name: String = srow.get("name");
            let target_state_raw: Option<String> = srow.get("target_state");
            let gates_raw: serde_json::Value = srow.get("gates");
            let params: serde_json::Value = srow.get("params");

            let target_state = target_state_raw.as_deref().and_then(state_from_db_str);
            let gates: Vec<GateSpec> = serde_json::from_value(gates_raw).unwrap_or_default();

            stages.push(StageSpec {
                name: stage_name,
                target_state,
                gates,
                params,
            });
        }

        workflows.insert(
            id.clone(),
            Workflow {
                workflow_id: id,
                name,
                trigger,
                stages,
                active,
                preset_id,
                workspace_install_ref,
            },
        );
    }

    Ok(workflows)
}

/// Mirror the workflow columns of an artifact to the DB after a stage
/// runner pass. Called alongside [`persist_artifact_state`] on transitions.
pub async fn persist_artifact_workflow_state(
    pool: &PgPool,
    artifact: &Artifact,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE artifacts \
            SET workflow_id = $1, \
                current_stage_index = $2, \
                workflow_parked_reason = $3 \
          WHERE artifact_id = $4",
    )
    .bind(artifact.workflow_id.as_deref())
    .bind(artifact.current_stage_index.map(|i| i as i32))
    .bind(artifact.workflow_parked_reason.as_deref())
    .bind(artifact.artifact_id.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_trigger(kind: &str, config: &serde_json::Value) -> Option<TriggerSpec> {
    match kind {
        "github_issue_webhook" => Some(TriggerSpec::GithubIssueWebhook {
            repo: config.get("repo")?.as_str()?.to_string(),
            label: config.get("label")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

fn state_from_db_str(s: &str) -> Option<ArtifactState> {
    match s {
        "draft" => Some(ArtifactState::Draft),
        "in_progress" => Some(ArtifactState::InProgress),
        "under_review" => Some(ArtifactState::UnderReview),
        "released" => Some(ArtifactState::Released),
        "archived" => Some(ArtifactState::Archived),
        _ => None,
    }
}

/// Inverse of [`state_from_db_str`]. Used by callers that want to write a
/// workflow row's `target_state` text without inventing the mapping.
pub fn state_to_db_str(state: ArtifactState) -> &'static str {
    state_to_db(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_issue_webhook_trigger() {
        let config = serde_json::json!({"repo": "a/b", "label": "ai"});
        let trigger = parse_trigger("github_issue_webhook", &config).expect("some");
        assert_eq!(
            trigger,
            TriggerSpec::GithubIssueWebhook {
                repo: "a/b".into(),
                label: "ai".into(),
            }
        );
    }

    #[test]
    fn rejects_unknown_trigger_kind() {
        let config = serde_json::json!({});
        assert!(parse_trigger("nonexistent_kind", &config).is_none());
    }

    #[test]
    fn rejects_trigger_with_missing_fields() {
        let config = serde_json::json!({"repo": "a/b"}); // no label
        assert!(parse_trigger("github_issue_webhook", &config).is_none());
    }

    #[test]
    fn state_roundtrip_strings() {
        for s in [
            ArtifactState::Draft,
            ArtifactState::InProgress,
            ArtifactState::UnderReview,
            ArtifactState::Released,
            ArtifactState::Archived,
        ] {
            assert_eq!(state_from_db_str(state_to_db_str(s)), Some(s));
        }
        assert_eq!(state_from_db_str("garbage"), None);
    }
}
