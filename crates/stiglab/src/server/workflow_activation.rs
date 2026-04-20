//! Workflow activation and deactivation hooks (issue #81).
//!
//! Activation:
//! 1. Resolve workspace install token (via `github_app::mint_installation_token`).
//! 2. Validate target repo is within the install scope.
//! 3. Ensure the configured trigger label exists on the repo; create if missing.
//! 4. Register the repo webhook for the required event types idempotently
//!    (dedup by URL).
//!
//! Deactivation:
//! - Deregister the repo webhook if no other active workflow on that repo
//!   still needs it.
//!
//! Every outbound GitHub call is bounded by a 10s timeout. All network paths
//! surface a typed error so the CRUD route can map them to a user-visible 4xx
//! (e.g. out-of-scope repo → 400) vs. a 5xx.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::server::github_app::{list_installation_repos, InstallationToken};

/// Typed errors from the activation path. The CRUD route uses these to map
/// install-scope rejections to 400 while bubbling everything else to 500.
#[derive(Debug, Error)]
pub enum ActivationError {
    #[error("workflow target repo {owner}/{repo} is outside the workspace install scope")]
    RepoOutOfScope { owner: String, repo: String },
    #[error("github api error: {0}")]
    GitHub(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Event types the webhook receiver cares about. Registered together so one
/// webhook covers every v1 routing rule.
pub const REQUIRED_WEBHOOK_EVENTS: &[&str] = &[
    "issues",
    "pull_request",
    "check_suite",
    "check_run",
    "status",
];

/// Public webhook URL used for idempotency deduping. The dashboard config
/// sets `STIGLAB_PUBLIC_BASE_URL` (or `STIGLAB_WEBHOOK_BASE_URL`) so the
/// registered URL matches the externally-exposed stiglab origin.
pub fn webhook_url() -> String {
    let base = std::env::var("STIGLAB_WEBHOOK_BASE_URL")
        .or_else(|_| std::env::var("STIGLAB_PUBLIC_BASE_URL"))
        .unwrap_or_else(|_| "http://localhost:3000".to_string());
    let base = base.trim_end_matches('/');
    format!("{base}/api/webhooks/github")
}

/// Verify a target repo is within an install's accessible-repo set.
pub async fn ensure_repo_in_scope(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
) -> Result<(), ActivationError> {
    let repos = list_installation_repos(token)
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if repos
        .iter()
        .any(|r| r.owner.eq_ignore_ascii_case(owner) && r.name.eq_ignore_ascii_case(repo))
    {
        Ok(())
    } else {
        Err(ActivationError::RepoOutOfScope {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

/// Ensure the configured trigger label exists on the target repo. Creates
/// the label via `POST /repos/:owner/:repo/labels` when absent. Idempotent.
pub async fn ensure_label_exists(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
    label: &str,
) -> Result<(), ActivationError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/labels/{label}");
    let resp = gh_client()
        .get(&url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "onsager-stiglab/0.1")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if resp.status().is_success() {
        return Ok(());
    }
    if resp.status().as_u16() != 404 {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ActivationError::GitHub(format!(
            "label lookup failed ({status}): {body}"
        )));
    }

    // Create the label. GitHub requires a `color` field; pick a neutral grey.
    let create_url = format!("https://api.github.com/repos/{owner}/{repo}/labels");
    let body = serde_json::json!({ "name": label, "color": "cfd8dc" });
    let resp = gh_client()
        .post(&create_url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "onsager-stiglab/0.1")
        .timeout(Duration::from_secs(10))
        .json(&body)
        .send()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if resp.status().is_success() {
        return Ok(());
    }
    // GitHub returns 422 when a label with that name already exists — treat
    // it as success for idempotency.
    if resp.status().as_u16() == 422 {
        return Ok(());
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    Err(ActivationError::GitHub(format!(
        "label create failed ({status}): {body}"
    )))
}

#[derive(Debug, Deserialize)]
struct HookRow {
    id: i64,
    #[serde(default)]
    config: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CreateHookBody<'a> {
    name: &'a str,
    events: &'a [&'a str],
    active: bool,
    config: CreateHookConfig<'a>,
}

#[derive(Debug, Serialize)]
struct CreateHookConfig<'a> {
    url: &'a str,
    content_type: &'a str,
    insecure_ssl: &'a str,
    secret: Option<&'a str>,
}

/// Register a repository webhook for the required events. Idempotent by
/// `config.url`: if a hook with the same URL already exists, no new hook is
/// created. Returns the hook id (new or existing).
pub async fn ensure_webhook_registered(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
    secret: Option<&str>,
) -> Result<i64, ActivationError> {
    let url = webhook_url();
    let list_url = format!("https://api.github.com/repos/{owner}/{repo}/hooks");
    let resp = gh_client()
        .get(&list_url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "onsager-stiglab/0.1")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ActivationError::GitHub(format!(
            "list hooks failed ({status}): {body}"
        )));
    }
    let hooks: Vec<HookRow> = resp
        .json()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if let Some(existing) = hooks.iter().find(|h| {
        h.config
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| u == url)
            .unwrap_or(false)
    }) {
        return Ok(existing.id);
    }

    let body = CreateHookBody {
        name: "web",
        events: REQUIRED_WEBHOOK_EVENTS,
        active: true,
        config: CreateHookConfig {
            url: &url,
            content_type: "json",
            insecure_ssl: "0",
            secret,
        },
    };
    let resp = gh_client()
        .post(&list_url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "onsager-stiglab/0.1")
        .timeout(Duration::from_secs(10))
        .json(&body)
        .send()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ActivationError::GitHub(format!(
            "create hook failed ({status}): {body}"
        )));
    }
    let created: HookRow = resp
        .json()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    Ok(created.id)
}

/// Remove the repository webhook pointing at our URL. No-op if nothing
/// matches. Safe to call when a deactivation decides no other active
/// workflow still needs the hook.
pub async fn deregister_webhook(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
) -> Result<(), ActivationError> {
    let url = webhook_url();
    let list_url = format!("https://api.github.com/repos/{owner}/{repo}/hooks");
    let resp = gh_client()
        .get(&list_url)
        .bearer_auth(&token.token)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "onsager-stiglab/0.1")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    if !resp.status().is_success() {
        return Ok(());
    }
    let hooks: Vec<HookRow> = match resp.json().await {
        Ok(h) => h,
        Err(_) => return Ok(()),
    };
    for h in hooks
        .iter()
        .filter(|h| h.config.get("url").and_then(|v| v.as_str()) == Some(url.as_str()))
    {
        let del_url = format!("https://api.github.com/repos/{owner}/{repo}/hooks/{}", h.id);
        let _ = gh_client()
            .delete(&del_url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "onsager-stiglab/0.1")
            .timeout(Duration::from_secs(10))
            .send()
            .await;
    }
    Ok(())
}

fn gh_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("onsager-stiglab/0.1")
            .build()
            .expect("failed to build activation client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_respects_base_env() {
        std::env::set_var("STIGLAB_WEBHOOK_BASE_URL", "https://stig.example.com/");
        assert_eq!(
            webhook_url(),
            "https://stig.example.com/api/webhooks/github"
        );
        std::env::remove_var("STIGLAB_WEBHOOK_BASE_URL");
    }

    #[test]
    fn required_events_include_v1_coverage() {
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"issues"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"pull_request"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"check_suite"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"check_run"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"status"));
    }
}
