//! `issues` event handler.
//!
//! Per spec #167 / #170, every `issues.opened` materializes a *reference-only*
//! `Kind::GithubIssue` skeleton artifact — no body, no labels, no title, no
//! author copied to the spine. Provider-authored fields are served live by
//! the `/api/projects/:id/issues` proxy in `crates/stiglab/src/server/routes/projects.rs`.
//! Lifecycle moves (`closed` / `reopened`) flip the skeleton's `state`;
//! everything else (`edited`, `labeled`, `unlabeled`, `assigned`, …) bumps
//! `current_version` and refreshes `last_observed_at` so ising sees activity
//! deltas without the spine carrying the actual change.
//!
//! This handler does not perform proxy-cache invalidation — the cache lives
//! in stiglab and freshness relies on its TTL window (default 60s).

use onsager_spine::EventMetadata;
use serde_json::Value;

use crate::db::{self, InstallationRecord, IssueLifecycleState};
use crate::state::AppState;

pub async fn handle(
    state: &AppState,
    installation: &InstallationRecord,
    payload: &Value,
) -> anyhow::Result<Value> {
    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    // Actions we react to: `opened` / `reopened` create or revive the
    // skeleton; `closed` archives it; the rest bump `current_version`.
    // Forward-compat: unknown actions ack quietly.
    let category = match action {
        "opened" | "reopened" => IssueAction::Open,
        "closed" => IssueAction::Close,
        "edited" | "labeled" | "unlabeled" | "assigned" | "unassigned" | "milestoned"
        | "demilestoned" | "pinned" | "unpinned" => IssueAction::Touch,
        _ => return Ok(serde_json::json!({"action": action, "ignored": true})),
    };

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

    let project =
        match db::find_project_for_repo(&state.pool, &installation.id, owner, name).await? {
            Some(p) => p,
            None => return Ok(serde_json::json!({"opted_in": false})),
        };

    let issue_number = issue
        .get("number")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing issue.number"))?;
    let github_state = issue
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("open");

    // Skeleton upsert. Open → draft, closed → archived. The proxy hydrates
    // title / labels / assignees on demand.
    let lifecycle = match category {
        IssueAction::Close => IssueLifecycleState::Archived,
        IssueAction::Open => IssueLifecycleState::Draft,
        IssueAction::Touch => IssueLifecycleState::from_github(github_state),
    };
    let artifact =
        db::upsert_issue_artifact_ref(&state.pool, &project.id, issue_number, lifecycle).await?;

    // For `Touch`, additionally bump `current_version` so ising sees an
    // activity delta. `upsert_issue_artifact_ref` already touches the
    // `last_observed_at` timestamp on every call.
    if matches!(category, IssueAction::Touch) {
        let _ = db::touch_artifact(&state.pool, &artifact.artifact_id).await?;
    }

    // The dashboard-facing live-hydration cache lives in stiglab; staleness
    // there is bounded by its TTL window (default 60s). Portal does not
    // need to push invalidations cross-process — TTL-bounded drift is
    // strictly better than denormalized writes (#170 design).

    // Emit a `portal.task_materialized` extension event so dashboard / ising
    // consumers see a uniform stream. The event name stays for back-compat
    // with #60 — only the payload reflects the new reference-only shape.
    let metadata = EventMetadata {
        actor: "onsager-portal".into(),
        ..Default::default()
    };
    let stream_id = format!("portal:issue:{}", artifact.artifact_id);
    state
        .spine
        .append_ext(
            &artifact.workspace_id,
            &stream_id,
            "portal",
            "portal.task_materialized",
            serde_json::json!({
                "artifact_id": artifact.artifact_id,
                "project_id": project.id,
                "external_ref": db::issue_external_ref(&project.id, issue_number),
                "issue_number": issue_number,
                "lifecycle": match lifecycle {
                    IssueLifecycleState::Draft => "draft",
                    IssueLifecycleState::Archived => "archived",
                },
                "action": action,
            }),
            &metadata,
            None,
        )
        .await?;

    Ok(serde_json::json!({
        "artifact_id": artifact.artifact_id,
        "project_id": project.id,
        "issue_number": issue_number,
        "lifecycle": match lifecycle {
            IssueLifecycleState::Draft => "draft",
            IssueLifecycleState::Archived => "archived",
        },
    }))
}

#[derive(Debug, Clone, Copy)]
enum IssueAction {
    Open,
    Close,
    Touch,
}
