//! GitHub App installation resolution for workflow activation (ADR 0024
//! item 3).
//!
//! `install_id` is a token-mint cache only — it does not participate in
//! lifecycle status. The authoritative binding is account/repo-level:
//! `workflow → (workspace_id, repo) → projects.github_app_installation_id`.
//! These helpers resolve the install through that path, falling back to the
//! workflow's cached `install_id` only when no matching project row exists
//! yet (e.g. the workflow was created before its repo was registered as a
//! project).

use crate::auth::decrypt_credential;
use crate::core::Project;
use crate::installation_db;
use crate::state::AppState;
use crate::workflow::Workflow;
use crate::workspace_db;

/// Resolve the GitHub App installation id for a workflow at activation time.
/// Prefers the project row matching the workflow's repo; falls back to the
/// workflow's cached `install_id`.
///
/// Returns `None` when neither path yields an install — the caller turns
/// that into the user-visible "no GitHub install for this repo" activation
/// error rather than minting against a stale id.
pub async fn resolve_workflow_install_id(state: &AppState, workflow: &Workflow) -> Option<i64> {
    let projects = if workflow.github_repo_any().is_some() {
        workspace_db::list_projects_for_workspace(&state.pool, &workflow.workspace_id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    pick_install_id(workflow, &projects)
}

/// Pure resolution decision for [`resolve_workflow_install_id`]: prefer the
/// project row that matches the workflow's repo (authoritative,
/// account-level), and fall back to the workflow's cached `install_id`
/// (mint cache) when no project matches. Kept separate so the
/// project-match-vs-fallback choice is unit-testable without a DB.
fn pick_install_id(workflow: &Workflow, projects: &[Project]) -> Option<i64> {
    if let Some((repo_owner, repo_name)) = workflow.github_repo_any()
        && let Some(project) = projects
            .iter()
            .find(|p| p.repo_owner == repo_owner && p.repo_name == repo_name)
        && let Ok(install_id) = project.github_app_installation_id.parse::<i64>()
    {
        return Some(install_id);
    }
    // Fallback: the cached mint id on the workflow row.
    workflow.install_id
}

/// Resolve and decrypt the per-install webhook signing secret.
pub async fn resolve_install_webhook_secret(state: &AppState, install_id: i64) -> Option<String> {
    let cipher = installation_db::get_install_webhook_secret_cipher(&state.pool, install_id)
        .await
        .ok()
        .flatten()
        .flatten()?;
    let key = state.config.credential_key.as_ref()?;
    decrypt_credential(key, &cipher).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::TriggerKind;
    use chrono::Utc;

    fn github_workflow(repo: &str, cached: Option<i64>) -> Workflow {
        let now = Utc::now();
        Workflow {
            id: "wf_1".into(),
            workspace_id: "w1".into(),
            name: "sdd".into(),
            trigger: TriggerKind::GithubIssueWebhook {
                repo: repo.into(),
                label: "spec".into(),
            },
            install_id: cached,
            preset_id: None,
            active: false,
            created_by: "u1".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn project(repo_owner: &str, repo_name: &str, install: &str) -> Project {
        Project {
            id: "p1".into(),
            workspace_id: "w1".into(),
            github_app_installation_id: install.into(),
            repo_owner: repo_owner.into(),
            repo_name: repo_name.into(),
            default_branch: "main".into(),
            created_at: Utc::now(),
        }
    }

    /// ADR 0024 item 3: install resolution prefers the project row matching
    /// the workflow's repo; the workflow's cached `install_id` is only a
    /// fallback. A matching project must win over the (possibly stale)
    /// cache — mutate `pick_install_id` to return `workflow.install_id`
    /// first and this fails.
    #[test]
    fn pick_install_id_prefers_matching_project_over_cache() {
        let wf = github_workflow("acme/widgets", Some(42));
        let projects = vec![project("acme", "widgets", "99")];
        assert_eq!(pick_install_id(&wf, &projects), Some(99));
    }

    #[test]
    fn pick_install_id_falls_back_to_cache_when_no_project_matches() {
        let wf = github_workflow("acme/widgets", Some(42));
        // Project for a different repo — no match, cache wins.
        let projects = vec![project("other", "repo", "99")];
        assert_eq!(pick_install_id(&wf, &projects), Some(42));
    }

    #[test]
    fn pick_install_id_none_when_no_project_and_no_cache() {
        let wf = github_workflow("acme/widgets", None);
        assert_eq!(pick_install_id(&wf, &[]), None);
    }

    #[test]
    fn pick_install_id_non_github_trigger_uses_cache_only() {
        let wf = Workflow {
            trigger: TriggerKind::Manual {
                name: "kickoff".into(),
            },
            install_id: None,
            ..github_workflow("unused/repo", None)
        };
        // A project for some repo must not be picked up by a Manual workflow.
        let projects = vec![project("acme", "widgets", "99")];
        assert_eq!(pick_install_id(&wf, &projects), None);
    }
}
