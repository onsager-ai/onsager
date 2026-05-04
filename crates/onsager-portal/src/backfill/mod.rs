//! Backfill — bulk-ingest a project's existing issues + PRs.
//!
//! Three strategies:
//!
//! - `recent` (default): the most-recent N items, paginated newest-first.
//!   Cheap; the right default for onboarding a fresh project.
//! - `active`: strips stale items (closed-but-unmerged PRs, closed issues)
//!   so a long-tail repo doesn't spend its cap on year-old chatter.
//! - `refract`: ranks the candidate set with a local priority heuristic
//!   (open-before-closed, more labels first) — the in-tree placeholder for
//!   the future LLM-backed scorer. Today it does NOT dispatch through the
//!   `refract` crate's intent decomposer; the crate is carried as a
//!   dependency so the scorer can land additively without a call-site
//!   refactor (issue #58 / #60 §Backfill).
//!
//! Output shape is a `BackfillReport` summarizing per-strategy counts so
//! the CLI can print a single JSON blob the dashboard can later render.

use std::str::FromStr;

use serde::Serialize;
use sqlx::postgres::PgPool;

use onsager_artifact::ArtifactId;
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::{EventMetadata, EventStore};

use onsager_github::api::issues::{list_recent_issues, Issue};
use onsager_github::api::pulls::{list_recent_pulls, Pull};

use crate::db::{self, IssueLifecycleState, PrLifecycleState};

/// Ingestion strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Recent,
    Active,
    Refract,
}

impl FromStr for Strategy {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "recent" => Ok(Strategy::Recent),
            "active" => Ok(Strategy::Active),
            "refract" => Ok(Strategy::Refract),
            other => anyhow::bail!("unknown strategy: {other}"),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct BackfillReport {
    pub strategy: String,
    pub project_id: String,
    pub repo: String,
    pub cap: usize,
    pub prs_ingested: usize,
    pub issues_ingested: usize,
    pub skipped: usize,
}

/// Drive the chosen strategy against the project's repo. Pulls candidates
/// from the GitHub REST API, then funnels them through the same write paths
/// the live webhook handler uses, so backfilled and live-streamed events
/// have identical shapes.
pub async fn run(
    pool: &PgPool,
    spine: &EventStore,
    project_id: &str,
    strategy: Strategy,
    cap: usize,
    github_token: Option<String>,
) -> anyhow::Result<BackfillReport> {
    let project = db::get_project(pool, project_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found"))?;
    let token = github_token.as_deref();
    let issues = list_recent_issues(token, &project.repo_owner, &project.repo_name, cap).await?;
    let pulls = list_recent_pulls(token, &project.repo_owner, &project.repo_name, cap).await?;

    let (issues, pulls) = match strategy {
        Strategy::Recent => (issues, pulls),
        Strategy::Active => (
            issues.into_iter().filter(|i| i.state == "open").collect(),
            pulls
                .into_iter()
                .filter(|p| p.state == "open" || p.merged_at.is_some())
                .collect(),
        ),
        Strategy::Refract => {
            // Placeholder ranking: open-before-closed, more-labelled first.
            // The real LLM-backed scorer slots in here without changing the
            // portal call site — today there is no `refract` crate API
            // doing prioritization; the dependency is pre-wired so the
            // scorer lands additively.
            (
                refract_prioritize_issues(issues, cap),
                refract_prioritize_pulls(pulls, cap),
            )
        }
    };

    let mut report = BackfillReport {
        strategy: format!("{strategy:?}").to_lowercase(),
        project_id: project.id.clone(),
        repo: format!("{}/{}", project.repo_owner, project.repo_name),
        cap,
        ..Default::default()
    };

    let metadata = EventMetadata {
        actor: "onsager-portal/backfill".into(),
        ..Default::default()
    };

    for pr in pulls {
        let next_state = match (pr.state.as_str(), pr.merged_at.is_some()) {
            (_, true) => PrLifecycleState::Released,
            ("closed", false) => PrLifecycleState::Archived,
            _ => PrLifecycleState::InProgress,
        };
        let artifact = db::upsert_pr_artifact_ref(pool, &project.id, pr.number, next_state).await?;
        let event = if let Some(merge_sha) = pr
            .merge_commit_sha
            .clone()
            .filter(|_| pr.merged_at.is_some())
        {
            // Only emit `GitPrMerged` when GitHub gave us the mainline merge
            // commit. `pr.head.sha` is the PR-branch tip, not the merge
            // commit — consumers correlating against `git log main` would
            // miss the match.
            FactoryEventKind::GitPrMerged {
                artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
                pr_number: pr.number,
                merge_sha,
            }
        } else if pr.state == "closed" {
            FactoryEventKind::GitPrClosed {
                artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
                pr_number: pr.number,
            }
        } else {
            FactoryEventKind::GitPrOpened {
                artifact_id: ArtifactId::new(artifact.artifact_id.clone()),
                repo: format!("{}/{}", project.repo_owner, project.repo_name),
                pr_number: pr.number,
                url: pr.html_url.clone(),
            }
        };
        let stream_id = crate::handlers::pull_request::pr_stream_id(&project.id, pr.number);
        spine
            .append_ext(
                &artifact.workspace_id,
                &stream_id,
                "git",
                event.event_type(),
                serde_json::to_value(&event)?,
                &metadata,
                None,
            )
            .await?;
        report.prs_ingested += 1;
    }

    for issue in issues {
        if issue.is_pull_request() {
            // PRs come back through the issues endpoint too; skip — they
            // were handled in the pulls loop.
            continue;
        }
        // Per spec #167: every issue becomes a reference-only skeleton
        // artifact. No label gate, no body copy.
        let lifecycle = IssueLifecycleState::from_github(&issue.state);
        let artifact =
            db::upsert_issue_artifact_ref(pool, &project.id, issue.number, lifecycle).await?;
        let stream_id = format!("portal:issue:{}", artifact.artifact_id);
        spine
            .append_ext(
                &artifact.workspace_id,
                &stream_id,
                "portal",
                "portal.task_materialized",
                serde_json::json!({
                    "artifact_id": artifact.artifact_id,
                    "project_id": project.id,
                    "external_ref": db::issue_external_ref(&project.id, issue.number),
                    "issue_number": issue.number,
                    "lifecycle": match lifecycle {
                        IssueLifecycleState::Draft => "draft",
                        IssueLifecycleState::Archived => "archived",
                    },
                    "backfill": true,
                }),
                &metadata,
                None,
            )
            .await?;
        report.issues_ingested += 1;
    }

    Ok(report)
}

/// Refract-style prioritization for issues. Today the heuristic is "open
/// before closed, more labels first" — a placeholder that demonstrates the
/// shape; the future LLM-backed scorer slots in here without changing the
/// portal call site.
fn refract_prioritize_issues(mut issues: Vec<Issue>, cap: usize) -> Vec<Issue> {
    issues.sort_by(|a, b| {
        let key = |i: &Issue| {
            let open = i.state == "open";
            let labels = i.labels.len();
            (open as i32, labels as i32)
        };
        key(b).cmp(&key(a))
    });
    issues.truncate(cap);
    issues
}

/// Refract-style prioritization for pulls. Open-then-merged-then-closed,
/// plus prefer base-branch=main as the most likely workflow signal.
fn refract_prioritize_pulls(mut pulls: Vec<Pull>, cap: usize) -> Vec<Pull> {
    pulls.sort_by(|a, b| {
        let key = |p: &Pull| {
            let open = p.state == "open";
            let merged = p.merged_at.is_some();
            let primary_base = p.base.ref_name == "main" || p.base.ref_name == "master";
            (open as i32, merged as i32, primary_base as i32)
        };
        key(b).cmp(&key(a))
    });
    pulls.truncate(cap);
    pulls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, state: &str, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue {number}"),
            state: state.into(),
            body: None,
            labels: labels
                .iter()
                .map(|n| onsager_github::api::issues::Label { name: (*n).into() })
                .collect(),
            pull_request: None,
        }
    }

    #[test]
    fn refract_orders_open_before_closed() {
        let mixed = vec![
            issue(1, "closed", &[]),
            issue(2, "open", &["bug"]),
            issue(3, "open", &["bug", "spec"]),
            issue(4, "closed", &["spec", "perf", "ux"]),
        ];
        let prioritized = refract_prioritize_issues(mixed, 4);
        // Most-labeled open first, then less-labeled open, then most-labeled closed.
        assert_eq!(prioritized[0].number, 3);
        assert_eq!(prioritized[1].number, 2);
        assert_eq!(prioritized[2].number, 4);
        assert_eq!(prioritized[3].number, 1);
    }

    #[test]
    fn refract_respects_cap() {
        let many: Vec<Issue> = (0..20).map(|n| issue(n, "open", &[])).collect();
        let cut = refract_prioritize_issues(many, 5);
        assert_eq!(cut.len(), 5);
    }

    #[test]
    fn strategy_parses() {
        assert_eq!("recent".parse::<Strategy>().unwrap(), Strategy::Recent);
        assert_eq!("ACTIVE".parse::<Strategy>().unwrap(), Strategy::Active);
        assert_eq!("Refract".parse::<Strategy>().unwrap(), Strategy::Refract);
        assert!("nope".parse::<Strategy>().is_err());
    }
}
