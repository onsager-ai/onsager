//! `issues` event handler.
//!
//! Two responsibilities in v1:
//! - `opened` / `labeled` carrying the `spec` label → materialize a `queued`
//!   factory task row + emit a portal-namespaced spine event.
//! - On the linked spec issue, mirror PR open/merge to add/remove the
//!   `in-progress` label (Phase 2 migration of `.github/workflows/pr-spec-sync.yml`)
//!   — that path is owned by `pull_request.rs`; this file only handles the
//!   issues side of materialization.

use onsager_spine::EventMetadata;
use serde_json::Value;

use crate::db::{self, InstallationRecord};
use crate::state::AppState;

/// Label whose presence on an issue triggers task materialization.
pub const SPEC_LABEL: &str = "spec";

pub async fn handle(
    state: &AppState,
    installation: &InstallationRecord,
    payload: &Value,
) -> anyhow::Result<Value> {
    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if action != "opened" && action != "labeled" {
        return Ok(serde_json::json!({"action": action, "ignored": true}));
    }

    let repo = payload
        .get("repository")
        .ok_or_else(|| anyhow::anyhow!("missing repository"))?;
    let owner = repo
        .get("owner")
        .and_then(|o| o.get("login"))
        .and_then(|l| l.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing repository.owner.login"))?;
    let name = repo
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing repository.name"))?;

    let issue = payload
        .get("issue")
        .ok_or_else(|| anyhow::anyhow!("missing issue"))?;

    // GitHub puts PRs into the `issues` API too — skip them; the
    // pull_request handler owns PR lifecycle.
    if issue.get("pull_request").is_some() {
        return Ok(serde_json::json!({"action": action, "ignored": "is_pr"}));
    }

    let labels: Vec<String> = issue
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let has_spec = labels.iter().any(|l| l == SPEC_LABEL);

    // For `labeled`, only materialize if the *added* label is `spec`. This
    // keeps materialization a single-shot transition rather than a no-op
    // every time someone touches labels on a spec issue.
    if action == "labeled" {
        let added = payload
            .get("label")
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        if added != SPEC_LABEL {
            return Ok(serde_json::json!({"action": action, "ignored": "not-spec-label"}));
        }
    }

    if !has_spec {
        return Ok(serde_json::json!({"action": action, "ignored": "no-spec-label"}));
    }

    let project =
        match db::find_project_for_repo(&state.pool, &installation.id, owner, name).await? {
            Some(p) => p,
            None => return Ok(serde_json::json!({"opted_in": false})),
        };

    let issue_number = issue
        .get("number")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing issue.number"))?;
    let title = issue
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("(untitled)")
        .to_string();
    let body = issue
        .get("body")
        .and_then(|b| b.as_str())
        .map(str::to_owned);

    let source_ref = format!("github:{owner}/{name}:issue#{issue_number}");
    let task = db::upsert_factory_task(
        &state.pool,
        &project.id,
        "spec_issue",
        &source_ref,
        &title,
        body.as_deref(),
    )
    .await?;

    // Emit a `portal.task_materialized` extension event so the dashboard /
    // ising can react. Stays under the `portal` namespace because it's a
    // backlog-state mutation, not a git event.
    let metadata = EventMetadata {
        actor: "onsager-portal".into(),
        ..Default::default()
    };
    let stream_id = format!("portal:task:{}", task.id);
    state
        .spine
        .append_ext(
            &stream_id,
            "portal",
            "portal.task_materialized",
            serde_json::json!({
                "task_id": task.id,
                "project_id": project.id,
                "source": task.source,
                "source_ref": task.source_ref,
                "title": task.title,
            }),
            &metadata,
            None,
        )
        .await?;

    Ok(serde_json::json!({
        "task_id": task.id,
        "project_id": project.id,
        "source_ref": task.source_ref,
    }))
}
