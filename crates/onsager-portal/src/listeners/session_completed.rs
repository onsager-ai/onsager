//! Spine listener for `stiglab.session_completed` → GitHub PR (spec #273).
//!
//! Portal is the only subsystem that holds GitHub App credentials, so it
//! owns the outbound GitHub API call. The happy-path sequence:
//!
//! 1. Consume `stiglab.session_completed`; extract `branch` + `artifact_id`.
//! 2. Look up the originating issue artifact to get project, repo, issue number.
//! 3. Mint an installation token via the GitHub App.
//! 4. Open a PR (`Closes #N`); materialize a `Kind::PullRequest` artifact in
//!    the spine; emit `git.pr_opened`; record horizontal lineage.
//!
//! Error and edge-case handling:
//! - No branch → silent skip (session did no git work).
//! - Empty branch → emit `portal.pr_open_failed` (`empty_branch`).
//! - `pr_number` already set on the event → session opened the PR itself;
//!   upsert the artifact and record lineage, no new GitHub call.
//! - Existing PR for (project, branch) → silent no-op per spec.
//! - GitHub API failure → emit `portal.pr_open_failed` (`github_api_error`).
//! - App not configured → emit `portal.pr_open_failed` (`app_not_configured`).

use std::sync::Arc;

use async_trait::async_trait;
use onsager_artifact::ArtifactId;
use onsager_github::api::app::{AppConfig, mint_app_jwt, mint_installation_token};
use onsager_github::api::pulls::{create_pull_request, find_open_pr_for_branch};
use onsager_spine::factory_event::FactoryEventKind;
use onsager_spine::{EventHandler, EventMetadata, EventNotification, EventStore, Listener};
use sqlx::postgres::PgPool;

use crate::db::{self, PrLifecycleState};

/// Run the session-completed → PR listener forever. Returns only when the
/// pg_notify channel closes.
pub async fn run(pool: PgPool, store: EventStore) -> anyhow::Result<()> {
    let handler = SessionPrOpener {
        pool: Arc::new(pool),
        store: store.clone(),
    };
    Listener::new(store).run(handler).await
}

struct SessionPrOpener {
    pool: Arc<PgPool>,
    store: EventStore,
}

impl SessionPrOpener {
    async fn handle_session_completed(
        &self,
        notification: &EventNotification,
    ) -> anyhow::Result<()> {
        // Load the typed event.
        let kind = match notification.table.as_str() {
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(());
                };
                serde_json::from_value::<FactoryEventKind>(row.data)?
            }
            _ => return Ok(()),
        };

        let FactoryEventKind::StiglabSessionCompleted {
            session_id,
            artifact_id: event_artifact_id,
            branch,
            pr_number: event_pr_number,
            ..
        } = kind
        else {
            return Ok(());
        };

        // No git work — skip silently.
        let Some(branch) = branch else {
            return Ok(());
        };

        // Empty branch pushed — error signal. Resolve workspace_id via artifact
        // lookup so the diagnostic event is scoped correctly; skip if unavailable.
        if branch.trim().is_empty() {
            if let Some(ref art_id) = event_artifact_id
                && let Ok(Some(art)) = db::find_artifact_info(&self.pool, art_id).await
            {
                self.emit_pr_open_failed(
                    Some(art_id),
                    Some(&branch),
                    &art.workspace_id,
                    "empty_branch",
                )
                .await;
            }
            return Ok(());
        }

        // Resolve the issue artifact. Without it we cannot find the repo.
        let Some(ref issue_artifact_id) = event_artifact_id else {
            tracing::debug!(
                session_id = %session_id,
                branch = %branch,
                "session_completed: no artifact_id, cannot open PR"
            );
            return Ok(());
        };

        let Some(art) = db::find_artifact_info(&self.pool, issue_artifact_id).await? else {
            tracing::warn!(
                session_id = %session_id,
                artifact_id = %issue_artifact_id,
                "session_completed: artifact not found"
            );
            return Ok(());
        };
        let workspace_id = art.workspace_id.clone();

        let Some(ref project_id) = art.project_id else {
            tracing::warn!(
                artifact_id = %issue_artifact_id,
                "session_completed: artifact has no project_id in metadata"
            );
            return Ok(());
        };

        let Some(project) = db::get_project(&self.pool, project_id).await? else {
            tracing::warn!(
                project_id = %project_id,
                "session_completed: project not found"
            );
            return Ok(());
        };

        let installation = crate::installation_db::get_installation(
            &self.pool,
            &project.github_app_installation_id,
        )
        .await?;
        let Some(install) = installation else {
            tracing::warn!(
                install_row_id = %project.github_app_installation_id,
                "session_completed: installation not found"
            );
            self.emit_pr_open_failed(
                Some(issue_artifact_id),
                Some(&branch),
                &workspace_id,
                "no_installation",
            )
            .await;
            return Ok(());
        };

        // If the event already carries a pr_number the agent opened the PR
        // itself during the session. Upsert the artifact and record lineage
        // without making a new GitHub API call.
        if let Some(pr_number) = event_pr_number {
            let pr_art = db::upsert_pr_artifact_ref(
                &self.pool,
                project_id,
                pr_number,
                PrLifecycleState::InProgress,
            )
            .await?;
            self.record_lineage_and_link(
                &pr_art.artifact_id,
                issue_artifact_id,
                art.current_version,
                &session_id,
                &branch,
                project_id,
                pr_number,
            )
            .await?;
            return Ok(());
        }

        // Idempotency: if a PR for this branch already exists in pr_branch_links,
        // silent no-op.
        if db::find_pr_for_branch(&self.pool, project_id, &branch)
            .await?
            .is_some()
        {
            return Ok(());
        }

        // Mint an installation token.
        let Some(app_cfg) = AppConfig::from_env() else {
            tracing::warn!("session_completed: GitHub App not configured, cannot open PR");
            self.emit_pr_open_failed(
                Some(issue_artifact_id),
                Some(&branch),
                &workspace_id,
                "app_not_configured",
            )
            .await;
            return Ok(());
        };
        let app_jwt = match mint_app_jwt(&app_cfg) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "session_completed: failed to mint GitHub App JWT");
                self.emit_pr_open_failed(
                    Some(issue_artifact_id),
                    Some(&branch),
                    &workspace_id,
                    "app_not_configured",
                )
                .await;
                return Ok(());
            }
        };
        let token = match mint_installation_token(&app_jwt, install.install_id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "session_completed: failed to mint installation token");
                self.emit_pr_open_failed(
                    Some(issue_artifact_id),
                    Some(&branch),
                    &workspace_id,
                    "github_api_error",
                )
                .await;
                return Ok(());
            }
        };

        // Before creating a new PR, check if GitHub already has an open PR for
        // this branch (handles the case where the previous listener run created
        // the PR but crashed before updating pr_branch_links).
        let existing = find_open_pr_for_branch(
            &token.token,
            &project.repo_owner,
            &project.repo_name,
            &branch,
        )
        .await;

        let (pr_number, pr_url) = match existing {
            Ok(Some(pr)) => {
                // PR already exists — no event emitted per spec.
                let pr_art = db::upsert_pr_artifact_ref(
                    &self.pool,
                    project_id,
                    pr.0,
                    PrLifecycleState::InProgress,
                )
                .await?;
                self.record_lineage_and_link(
                    &pr_art.artifact_id,
                    issue_artifact_id,
                    art.current_version,
                    &session_id,
                    &branch,
                    project_id,
                    pr.0,
                )
                .await?;
                return Ok(());
            }
            Ok(None) => {
                // Build the PR title and body.
                let title = art
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| {
                        art.issue_number
                            .map(|n| format!("Update for issue #{n}"))
                            .unwrap_or_else(|| "Update from Onsager agent".to_string())
                    });
                let body = art
                    .issue_number
                    .map(|n| format!("Closes #{n}\n\n"))
                    .unwrap_or_default();

                match create_pull_request(
                    &token.token,
                    &project.repo_owner,
                    &project.repo_name,
                    &title,
                    &body,
                    &branch,
                    &project.default_branch,
                )
                .await
                {
                    Ok(pr) => pr,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            branch = %branch,
                            repo = %format!("{}/{}", project.repo_owner, project.repo_name),
                            "session_completed: GitHub create_pull_request failed"
                        );
                        self.emit_pr_open_failed(
                            Some(issue_artifact_id),
                            Some(&branch),
                            &workspace_id,
                            "github_api_error",
                        )
                        .await;
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "session_completed: find_open_pr_for_branch failed");
                // Proceed with creation attempt; GitHub will 422 if it already exists.
                let title = art
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| {
                        art.issue_number
                            .map(|n| format!("Update for issue #{n}"))
                            .unwrap_or_else(|| "Update from Onsager agent".to_string())
                    });
                let body = art
                    .issue_number
                    .map(|n| format!("Closes #{n}\n\n"))
                    .unwrap_or_default();

                match create_pull_request(
                    &token.token,
                    &project.repo_owner,
                    &project.repo_name,
                    &title,
                    &body,
                    &branch,
                    &project.default_branch,
                )
                .await
                {
                    Ok(pr) => pr,
                    Err(e) => {
                        tracing::warn!(error = %e, "session_completed: create_pull_request failed");
                        self.emit_pr_open_failed(
                            Some(issue_artifact_id),
                            Some(&branch),
                            &workspace_id,
                            "github_api_error",
                        )
                        .await;
                        return Ok(());
                    }
                }
            }
        };

        // Materialize the PR artifact and record lineage.
        let pr_art = db::upsert_pr_artifact_ref(
            &self.pool,
            project_id,
            pr_number,
            PrLifecycleState::InProgress,
        )
        .await?;

        self.record_lineage_and_link(
            &pr_art.artifact_id,
            issue_artifact_id,
            art.current_version,
            &session_id,
            &branch,
            project_id,
            pr_number,
        )
        .await?;

        // Emit git.pr_opened.
        let event = FactoryEventKind::GitPrOpened {
            artifact_id: ArtifactId::new(pr_art.artifact_id.clone()),
            repo: format!("{}/{}", project.repo_owner, project.repo_name),
            pr_number,
            url: pr_url.clone(),
        };
        let stream_id = crate::handlers::pull_request::pr_stream_id(project_id, pr_number);
        let metadata = EventMetadata {
            actor: "onsager-portal".into(),
            ..Default::default()
        };
        if let Err(e) = self
            .store
            .append_ext(
                &workspace_id,
                &stream_id,
                "git",
                event.event_type(),
                serde_json::to_value(&event)?,
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!(error = %e, "session_completed: failed to emit git.pr_opened");
        }

        tracing::info!(
            session_id = %session_id,
            pr_number,
            pr_url = %pr_url,
            "session_completed: PR opened"
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_lineage_and_link(
        &self,
        pr_artifact_id: &str,
        issue_artifact_id: &str,
        issue_version: i32,
        session_id: &str,
        branch: &str,
        project_id: &str,
        pr_number: u64,
    ) -> anyhow::Result<()> {
        // Horizontal lineage: PR artifact → issue artifact.
        if let Err(e) = db::record_horizontal_lineage(
            &self.pool,
            pr_artifact_id,
            issue_artifact_id,
            issue_version,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to record horizontal lineage");
        }
        // Session↔PR branch link for idempotency on retries.
        if let Err(e) = db::record_session_branch(
            &self.pool,
            session_id,
            Some(project_id),
            branch,
            Some(pr_number),
        )
        .await
        {
            tracing::warn!(error = %e, "failed to record session branch link");
        }
        Ok(())
    }

    async fn emit_pr_open_failed(
        &self,
        artifact_id: Option<&str>,
        branch: Option<&str>,
        workspace_id: &str,
        reason: &str,
    ) {
        let event = FactoryEventKind::PortalPrOpenFailed {
            artifact_id: artifact_id.map(|id| ArtifactId::new(id.to_string())),
            branch: branch.map(|b| b.to_string()),
            workspace_id: workspace_id.to_string(),
            reason: reason.to_string(),
        };
        let stream_id = event.stream_id();
        let metadata = EventMetadata {
            actor: "onsager-portal".into(),
            ..Default::default()
        };
        if let Err(e) = self
            .store
            .append_ext(
                workspace_id,
                &stream_id,
                "portal",
                event.event_type(),
                serde_json::to_value(&event).unwrap_or_default(),
                &metadata,
                None,
            )
            .await
        {
            tracing::warn!(error = %e, "failed to emit portal.pr_open_failed");
        }
    }
}

#[async_trait]
impl EventHandler for SessionPrOpener {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "stiglab.session_completed" {
            return Ok(());
        }
        if let Err(e) = self.handle_session_completed(&notification).await {
            tracing::warn!(
                error = %e,
                event_id = notification.id,
                "session_completed listener: handler error"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `run` function accepts the correct types; compiling this catches
    /// signature drift without needing a live DB.
    #[allow(dead_code)]
    fn _type_check_run_signature(pool: PgPool, store: EventStore) {
        std::mem::drop(run(pool, store));
    }
}
