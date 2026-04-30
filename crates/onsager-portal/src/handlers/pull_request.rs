//! `pull_request` event handler.
//!
//! Translates the relevant actions (`opened`, `reopened`, `synchronize`,
//! `closed`) into:
//! - PR-artifact upserts under `Kind::PullRequest`
//! - `git.pr_*` rows on `events_ext` keyed by `git:{project_id}:pr:{number}`
//! - vertical lineage when the PR's `head.ref` matches a recent session
//! - synodic gate evaluation per `(pr_artifact_id, head_sha)` (Phase 2)

use onsager_artifact::ArtifactId;
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::EventMetadata;
use serde_json::Value;

use onsager_github::api::pulls::CheckConclusion;

use crate::db::{self, InstallationRecord, PrLifecycleState};
use crate::gate::{GateInput, Verdict};
use crate::state::AppState;

const GIT_NAMESPACE: &str = "git";

/// Action strings the handler responds to. Anything else is acknowledged
/// without side effects so handlers stay forward-compatible.
fn map_action(action: &str) -> Option<PrAction> {
    match action {
        "opened" | "reopened" => Some(PrAction::Opened),
        "synchronize" => Some(PrAction::Synchronize),
        "closed" => Some(PrAction::Closed),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum PrAction {
    Opened,
    Synchronize,
    Closed,
}

pub async fn handle(
    state: &AppState,
    installation: &InstallationRecord,
    payload: &Value,
) -> anyhow::Result<Value> {
    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let Some(act) = map_action(action) else {
        return Ok(serde_json::json!({"action": action, "ignored": true}));
    };

    // Resolve repo owner/name from the payload — `repository.owner.login`
    // and `repository.name`. Without these the event isn't routable.
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

    let project =
        match db::find_project_for_repo(&state.pool, &installation.id, owner, name).await? {
            Some(p) => p,
            None => {
                // The repo is reachable by the installation but the tenant hasn't
                // opted it in. Acknowledge the delivery so GitHub stops retrying.
                return Ok(serde_json::json!({"opted_in": false}));
            }
        };

    let pr = payload
        .get("pull_request")
        .ok_or_else(|| anyhow::anyhow!("missing pull_request"))?;
    let pr_number = pr
        .get("number")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing pr.number"))?;
    // PR title is fetched live by the proxy; the message field on
    // `GitCommitPushed` keeps a synchronize-time snapshot purely for the
    // spine event payload (events are immutable historical facts, not a
    // denormalization of current state — the artifact row stays clean).
    let sync_message = pr
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let url = pr
        .get("html_url")
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();
    let head_sha = pr
        .get("head")
        .and_then(|h| h.get("sha"))
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    let head_ref = pr
        .get("head")
        .and_then(|h| h.get("ref"))
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    let merged = pr.get("merged").and_then(|m| m.as_bool()).unwrap_or(false);
    let body = pr
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("")
        .to_string();
    let merge_sha = pr
        .get("merge_commit_sha")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let next_state = match (act, merged) {
        (PrAction::Opened, _) => PrLifecycleState::InProgress,
        (PrAction::Synchronize, _) => PrLifecycleState::InProgress,
        (PrAction::Closed, true) => PrLifecycleState::Released,
        (PrAction::Closed, false) => PrLifecycleState::Archived,
    };

    // Reference-only upsert (#171): no `name` / `owner` writes — the proxy
    // hydrates those from GitHub on dashboard render.
    let artifact =
        db::upsert_pr_artifact_ref(&state.pool, &project.id, pr_number, next_state).await?;

    // Bump version on synchronize / close so the artifact's lifecycle reflects
    // commit-by-commit progress (powers ising's PR-churn signal per #62).
    if matches!(act, PrAction::Synchronize | PrAction::Closed) {
        let _ = db::bump_pr_artifact(&state.pool, &artifact.artifact_id, Some(next_state)).await?;
    }

    // The dashboard-facing live-hydration cache lives in stiglab; staleness
    // there is bounded by its TTL window. Portal does not need to push
    // invalidations cross-process.

    // Emit the matching spine event under namespace `git`.
    let stream_id = pr_stream_id(&project.id, pr_number);
    let event = match act {
        PrAction::Opened => FactoryEventKind::GitPrOpened {
            artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
            repo: format!("{owner}/{name}"),
            pr_number,
            url: url.clone(),
        },
        PrAction::Synchronize => FactoryEventKind::GitCommitPushed {
            artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
            sha: head_sha.clone(),
            message: sync_message.clone(),
            session_id: String::new(),
        },
        PrAction::Closed if merged => FactoryEventKind::GitPrMerged {
            artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
            pr_number,
            merge_sha: merge_sha.clone(),
        },
        PrAction::Closed => FactoryEventKind::GitPrClosed {
            artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
            pr_number,
        },
    };

    let metadata = EventMetadata {
        actor: "onsager-portal".into(),
        ..Default::default()
    };
    let event_id = state
        .spine
        .append_ext(
            &stream_id,
            GIT_NAMESPACE,
            event.event_type(),
            serde_json::to_value(&event)?,
            &metadata,
            None,
        )
        .await?;

    // Session↔PR correlation: on `opened`, attach a vertical_lineage row if a
    // recent session pushed `head.ref` against this project (Phase 1).
    if matches!(act, PrAction::Opened) && !head_ref.is_empty() {
        if let Some(session_id) =
            db::find_session_for_branch(&state.pool, &project.id, &head_ref).await?
        {
            db::link_session_to_pr_artifact(
                &state.pool,
                &artifact.artifact_id,
                &session_id,
                artifact.current_version,
            )
            .await?;
        }
    }

    // Phase 2: gate the commit and post a check run.
    let mut gate_outcome: Option<Verdict> = None;
    if matches!(act, PrAction::Opened | PrAction::Synchronize) && !head_sha.is_empty() {
        let existing =
            db::find_existing_verdict(&state.pool, &artifact.artifact_id, &head_sha).await?;
        let verdict = if let Some(prior) = existing {
            // Idempotent dedup: same SHA, same verdict — nothing new to emit.
            tracing::debug!(verdict = %prior, sha = %head_sha, "skipping duplicate gate");
            None
        } else {
            let v = state
                .gate
                .evaluate(&GateInput {
                    artifact_id: artifact.artifact_id.clone(),
                    artifact_kind: "pull_request".into(),
                    current_state: "in_progress".into(),
                    head_sha: head_sha.clone(),
                })
                .await;
            db::record_verdict(
                &state.pool,
                &artifact.artifact_id,
                &head_sha,
                v.as_summary(),
            )
            .await?;
            Some(v)
        };

        if let Some(v) = verdict {
            // Emit `forge.gate_verdict` mirroring forge's emission shape so
            // ising and `/governance` see a uniform stream.
            let verdict_summary = v.as_summary();
            let gate_event = serde_json::json!({
                "artifact_id": artifact.artifact_id,
                "gate_point": "state_transition",
                "verdict": verdict_summary,
                "head_sha": head_sha,
                "project_id": project.id,
            });
            state
                .spine
                .append_ext(
                    &artifact.artifact_id,
                    "forge",
                    "forge.gate_verdict",
                    gate_event,
                    &metadata,
                    Some(event_id),
                )
                .await?;

            // Best-effort GitHub check run. Only attempted when a token is
            // configured — installation-token signing is a Phase 2 follow-up.
            if let Some(tok) = state.config.github_token.as_deref() {
                let (conclusion, summary) = match &v {
                    Verdict::Allow => (
                        CheckConclusion::Success,
                        "synodic/gate: no rules apply (allow)".to_string(),
                    ),
                    Verdict::Deny { reason } => {
                        // Post the rationale as a review comment so the
                        // contributor sees why the gate failed.
                        if let Err(e) = onsager_github::api::issues::post_issue_comment(
                            Some(tok),
                            owner,
                            name,
                            pr_number,
                            &format!("**synodic/gate denied**: {reason}"),
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Deny comment post failed");
                        }
                        (
                            CheckConclusion::Failure,
                            format!("synodic/gate: deny — {reason}"),
                        )
                    }
                    Verdict::Modify => (
                        CheckConclusion::Neutral,
                        "synodic/gate: modify (informational)".to_string(),
                    ),
                    Verdict::Escalate { reason } => (
                        CheckConclusion::ActionRequired,
                        format!("synodic/gate: escalate — {reason}"),
                    ),
                };
                if let Err(e) = onsager_github::api::pulls::create_check_run(
                    Some(tok),
                    owner,
                    name,
                    &head_sha,
                    "synodic/gate",
                    conclusion,
                    &summary,
                )
                .await
                {
                    tracing::warn!(error = %e, "check-run post failed");
                }
            }

            // Escalate produces a synodic governance event so the dashboard
            // can show the parked decision.
            if let Verdict::Escalate { reason } = &v {
                let escalation_payload = serde_json::json!({
                    "escalation_id": format!("esc_pr_{}_{}", artifact.artifact_id, head_sha),
                    "artifact_id": artifact.artifact_id,
                    "reason": reason,
                    "source": "onsager-portal",
                });
                state
                    .spine
                    .append_ext(
                        &artifact.artifact_id,
                        "synodic",
                        "synodic.escalation_started",
                        escalation_payload,
                        &metadata,
                        Some(event_id),
                    )
                    .await?;
            }

            gate_outcome = Some(v);
        }
    }

    // Phase 2 label-sync migration (`.github/workflows/pr-spec-sync.yml`).
    // Best-effort: only attempted when a github_token is configured. Failures
    // are logged so the workflow can stay live as a safety net during rollout.
    if let Some(tok) = state.config.github_token.as_deref() {
        if !body.is_empty() {
            let linked = crate::handlers::spec_link::linked_issues(&body);
            if !linked.is_empty() {
                use onsager_github::api::issues::set_label;
                for issue_number in linked {
                    let result = match (act, merged) {
                        (PrAction::Opened, _) => {
                            set_label(Some(tok), owner, name, issue_number, "in-progress", true)
                                .await
                        }
                        (PrAction::Closed, true) => {
                            // PR merged — move spec from `in-progress` to `done`
                            // to mirror the human-driven label progression.
                            let _ = set_label(
                                Some(tok),
                                owner,
                                name,
                                issue_number,
                                "in-progress",
                                false,
                            )
                            .await;
                            set_label(Some(tok), owner, name, issue_number, "done", true).await
                        }
                        (PrAction::Closed, false) => {
                            // Closed without merge — revert to `planned` so the
                            // spec re-enters the queue cleanly.
                            let _ = set_label(
                                Some(tok),
                                owner,
                                name,
                                issue_number,
                                "in-progress",
                                false,
                            )
                            .await;
                            set_label(Some(tok), owner, name, issue_number, "planned", true).await
                        }
                        (PrAction::Synchronize, _) => Ok(()),
                    };
                    if let Err(e) = result {
                        tracing::warn!(error = %e, issue = issue_number, "spec label sync failed");
                    }
                }
            }
        }
    }

    Ok(serde_json::json!({
        "project_id": project.id,
        "artifact_id": artifact.artifact_id,
        "pr_number": pr_number,
        "event_id": event_id,
        "gate": gate_outcome.map(|v| v.as_summary().to_string()),
    }))
}

/// Stream id for `events_ext`, scoped per project so the same PR number
/// across two projects never collides.
pub fn pr_stream_id(project_id: &str, pr_number: u64) -> String {
    format!("git:{project_id}:pr:{pr_number}")
}
