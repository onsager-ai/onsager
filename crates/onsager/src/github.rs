//! GitHub integration: the App webhook receiver, trigger matching, and
//! the run's repo-access token (M2 #624).
//!
//! One webhook for the whole App (configured once in the App settings,
//! verified with `GITHUB_WEBHOOK_SECRET`) — no per-repo registration,
//! no per-install secrets (decision on #624). `installation` events
//! maintain the installations table; `issues.labeled` events match
//! active workflows by repo + label and fire them with the issue as
//! trigger context.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use onsager_github::api::app::{AppConfig, mint_app_jwt, mint_repo_write_token};
use onsager_github::webhook::{SignatureCheck, verify_signature};
use sqlx::PgPool;

use crate::domain::{StageKind, TriggerKind, Workflow};
use crate::sessions::ClonableRepo;
use crate::state::AppState;

/// `POST /api/webhooks/github` — the App's webhook. Fails closed when
/// the secret is unconfigured.
pub async fn receive(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(secret) = state.config.github_webhook_secret.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook secret not configured",
        )
            .into_response();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    match verify_signature(signature, &body, secret.as_bytes()) {
        SignatureCheck::Valid => {}
        _ => return (StatusCode::UNAUTHORIZED, "bad signature").into_response(),
    }

    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "bad json").into_response(),
    };

    match event_type {
        "installation" => handle_installation(&state.pool, &payload).await,
        "issues" => handle_issues(&state, &payload).await,
        // Anything else the App is subscribed to is acknowledged and
        // ignored — new event types land with their consumers.
        _ => {}
    }
    (StatusCode::ACCEPTED, "ok").into_response()
}

async fn handle_installation(pool: &PgPool, payload: &serde_json::Value) {
    let action = payload["action"].as_str().unwrap_or("");
    let install_id = payload["installation"]["id"].as_i64().unwrap_or(0);
    if install_id == 0 {
        return;
    }
    let result = match action {
        "created" | "unsuspend" => {
            let login = payload["installation"]["account"]["login"]
                .as_str()
                .unwrap_or("");
            let kind = payload["installation"]["account"]["type"]
                .as_str()
                .unwrap_or("Organization");
            sqlx::query(
                "INSERT INTO github_app_installations (installation_id, account_login, account_kind) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (installation_id) DO UPDATE SET account_login = $2, account_kind = $3",
            )
            .bind(install_id)
            .bind(login)
            .bind(kind)
            .execute(pool)
            .await
        }
        "deleted" | "suspend" => {
            sqlx::query("DELETE FROM github_app_installations WHERE installation_id = $1")
                .bind(install_id)
                .execute(pool)
                .await
        }
        _ => return,
    };
    if let Err(e) = result {
        tracing::error!("installation webhook upsert failed: {e}");
    } else {
        tracing::info!(install_id, action, "github installation updated");
    }
}

async fn handle_issues(state: &AppState, payload: &serde_json::Value) {
    if payload["action"].as_str() != Some("labeled") {
        return;
    }
    let repo = payload["repository"]["full_name"].as_str().unwrap_or("");
    let label = payload["label"]["name"].as_str().unwrap_or("");
    if repo.is_empty() || label.is_empty() {
        return;
    }
    let workflows: Vec<Workflow> = match sqlx::query_as(
        "SELECT id, workspace_id, name, trigger, definition, active, created_by, \
                created_at, updated_at \
         FROM workflows WHERE active = TRUE",
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("webhook workflow scan failed: {e}");
            return;
        }
    };
    let context = issue_context(payload);
    for wf in workflows {
        if !matches_issue_trigger(&wf, repo, label) {
            continue;
        }
        tracing::info!(workflow_id = %wf.id, repo, label, "issue trigger matched");
        let fired = with_trigger_context(wf, &context);
        if let Err(e) = crate::scheduler::fire(state, fired).await {
            tracing::error!("webhook fire failed: {e}");
        }
    }
}

/// Pure trigger match — repo slug and label must both equal the
/// workflow's `github_issue_webhook` config.
pub fn matches_issue_trigger(workflow: &Workflow, repo: &str, label: &str) -> bool {
    match &workflow.trigger {
        TriggerKind::GithubIssueWebhook {
            repo: wf_repo,
            label: wf_label,
        } => wf_repo.eq_ignore_ascii_case(repo) && wf_label == label,
        TriggerKind::Manual { .. } => false,
    }
}

/// The issue, rendered as trigger context for the agent (#624: trigger
/// context rides the prompt until artifact inputs are a feature).
fn issue_context(payload: &serde_json::Value) -> String {
    let number = payload["issue"]["number"].as_u64().unwrap_or(0);
    let title = payload["issue"]["title"].as_str().unwrap_or("");
    let url = payload["issue"]["html_url"].as_str().unwrap_or("");
    let body = payload["issue"]["body"].as_str().unwrap_or("");
    format!("# Trigger: GitHub issue #{number}\n\nTitle: {title}\nURL: {url}\n\n{body}\n\n---\n\n")
}

/// Prepend the trigger context to the first agent stage's prompt.
pub fn with_trigger_context(mut workflow: Workflow, context: &str) -> Workflow {
    if context.is_empty() {
        return workflow;
    }
    if let Some(StageKind::Agent { user_prompt, .. }) = workflow.definition.first_mut() {
        *user_prompt = format!("{context}{user_prompt}");
    }
    workflow
}

/// The run's repo-access set: a write-scoped installation token for the
/// trigger repo (binding the App is the consent — #624), shaped as
/// [`ClonableRepo`] for the `ONSAGER_REPOS` env. Returns empty when the
/// workflow has no GitHub surface or the App is unconfigured (the run
/// proceeds without repo access; a private clone then fails loudly).
pub async fn repo_access_for(pool: &PgPool, workflow: &Workflow) -> Vec<ClonableRepo> {
    let Some(slug) = workflow.trigger.github_repo() else {
        return Vec::new();
    };
    let Some((owner, name)) = slug.split_once('/') else {
        tracing::warn!(slug, "trigger repo is not owner/name");
        return Vec::new();
    };
    let Some(cfg) = AppConfig::from_env() else {
        tracing::warn!("GitHub App unconfigured — run proceeds without repo token");
        return vec![ClonableRepo {
            owner: owner.into(),
            name: name.into(),
            default_branch: "main".into(),
            token: None,
        }];
    };
    let install: Option<(i64,)> = match sqlx::query_as(
        "SELECT installation_id FROM github_app_installations WHERE account_login = $1",
    )
    .bind(owner)
    .fetch_optional(pool)
    .await
    {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("installation lookup failed: {e}");
            None
        }
    };
    let token = match install {
        Some((install_id,)) => match mint_app_jwt(&cfg) {
            Ok(jwt) => match mint_repo_write_token(&jwt, install_id, &[name.to_string()]).await {
                Ok(t) => Some(t.token),
                Err(e) => {
                    tracing::warn!("repo token mint failed: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("app jwt mint failed: {e}");
                None
            }
        },
        None => {
            tracing::warn!(owner, "no installation for trigger repo's account");
            None
        }
    };
    vec![ClonableRepo {
        owner: owner.into(),
        name: name.into(),
        default_branch: "main".into(),
        token,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn wf(trigger: TriggerKind) -> Workflow {
        Workflow {
            id: "wf_t".into(),
            workspace_id: "ws".into(),
            name: "t".into(),
            trigger,
            definition: vec![StageKind::Agent {
                model: None,
                system_prompt: None,
                user_prompt: "fix the issue".into(),
            }],
            active: true,
            created_by: "u".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn issue_trigger_matches_repo_and_label_only() {
        let w = wf(TriggerKind::GithubIssueWebhook {
            repo: "acme/widgets".into(),
            label: "spec".into(),
        });
        assert!(matches_issue_trigger(&w, "acme/widgets", "spec"));
        assert!(
            matches_issue_trigger(&w, "ACME/widgets", "spec"),
            "repo is case-insensitive"
        );
        assert!(!matches_issue_trigger(&w, "acme/widgets", "bug"));
        assert!(!matches_issue_trigger(&w, "acme/other", "spec"));
        assert!(!matches_issue_trigger(
            &wf(TriggerKind::Manual { name: "go".into() }),
            "acme/widgets",
            "spec"
        ));
    }

    #[test]
    fn trigger_context_prepends_to_first_agent_stage() {
        let w = wf(TriggerKind::Manual { name: "go".into() });
        let fired = with_trigger_context(w, "# Trigger: issue #7\n\n");
        let StageKind::Agent { user_prompt, .. } = &fired.definition[0];
        assert!(user_prompt.starts_with("# Trigger: issue #7"));
        assert!(user_prompt.ends_with("fix the issue"));
    }
}
