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

use serde::Serialize;
use thiserror::Error;

use crate::server::github_app::{list_installation_repos, InstallationToken};

/// Typed errors from the activation path. The CRUD route uses these to map
/// install-scope rejections to 400 while bubbling everything else to 500.
#[derive(Debug, Error)]
pub enum ActivationError {
    #[error("workflow target repo {owner}/{repo} is outside the workspace install scope")]
    RepoOutOfScope { owner: String, repo: String },
    /// The GitHub App install responded with a 403 "Resource not accessible by
    /// integration" — the App manifest is missing the permission required for
    /// `action` (or the install hasn't accepted an updated permission set).
    /// The CRUD route maps this to a user-visible 4xx so the dashboard can
    /// prompt the operator to update the App's permissions, rather than the
    /// generic 502 we use for opaque upstream failures.
    #[error("github app install is missing permission for {action}")]
    MissingGithubPermission { action: String, details: String },
    #[error("github api error: {0}")]
    GitHub(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Classify a non-2xx response from the GitHub REST API. Returns a
/// `MissingGithubPermission` when the status + body match the signature of a
/// missing App permission (so the caller surfaces a dashboard-actionable 4xx);
/// otherwise falls back to the opaque `GitHub` variant.
fn classify_github_error(action: &str, status: reqwest::StatusCode, body: &str) -> ActivationError {
    if status.as_u16() == 403 && body.contains("Resource not accessible by integration") {
        return ActivationError::MissingGithubPermission {
            action: action.to_string(),
            details: format!(
                "GitHub App install is missing the 'Repository webhooks: Read & write' \
                 permission required to {action}. Update the App's permissions on GitHub \
                 and accept the new permission request on the installation, then retry."
            ),
        };
    }
    ActivationError::GitHub(format!("{action} failed ({status}): {body}"))
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
    // Label names may legitimately contain spaces or other characters that
    // need percent-encoding when interpolated into the URL path. Build the
    // URL with `reqwest::Url::path_segments_mut` rather than raw `format!`.
    let mut url = reqwest::Url::parse(&format!(
        "https://api.github.com/repos/{owner}/{repo}/labels"
    ))
    .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    url.path_segments_mut()
        .map_err(|_| ActivationError::GitHub("invalid GitHub label lookup URL".into()))?
        .push(label);
    let resp = gh_client()
        .get(url)
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
        return Err(classify_github_error("look up repo labels", status, &body));
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
    Err(classify_github_error(
        "create the trigger label",
        status,
        &body,
    ))
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
/// `config.url`: if a hook with the same URL already exists, it is reused.
/// When the existing hook is disabled or missing any of `REQUIRED_WEBHOOK_EVENTS`
/// we `PATCH` it in place so the workflow runtime doesn't silently miss
/// deliveries. Returns the hook id (new, patched, or unchanged).
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
        return Err(classify_github_error(
            "list the repo webhooks",
            status,
            &body,
        ));
    }
    let hooks: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    let required_events: std::collections::BTreeSet<&str> =
        REQUIRED_WEBHOOK_EVENTS.iter().copied().collect();
    if let Some(existing) = hooks.iter().find(|h| {
        h.get("config")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .map(|u| u == url)
            .unwrap_or(false)
    }) {
        let existing_id = existing
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ActivationError::GitHub("existing hook missing numeric id".into()))?;
        let is_active = existing
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let existing_events: std::collections::BTreeSet<&str> = existing
            .get("events")
            .and_then(|v| v.as_array())
            .map(|events| events.iter().filter_map(|e| e.as_str()).collect())
            .unwrap_or_default();
        if is_active && existing_events == required_events {
            return Ok(existing_id);
        }

        // Hook exists but is disabled or missing events — PATCH it in place.
        let patch_url = format!("{list_url}/{existing_id}");
        let patch_body = serde_json::json!({
            "active": true,
            "events": REQUIRED_WEBHOOK_EVENTS,
        });
        let resp = gh_client()
            .patch(&patch_url)
            .bearer_auth(&token.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "onsager-stiglab/0.1")
            .timeout(Duration::from_secs(10))
            .json(&patch_body)
            .send()
            .await
            .map_err(|e| ActivationError::GitHub(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_github_error(
                "update the repo webhook",
                status,
                &body,
            ));
        }
        return Ok(existing_id);
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
        return Err(classify_github_error(
            "create the repo webhook",
            status,
            &body,
        ));
    }
    let created: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ActivationError::GitHub(e.to_string()))?;
    created
        .get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ActivationError::GitHub("created hook missing numeric id".into()))
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
    let hooks: Vec<serde_json::Value> = match resp.json().await {
        Ok(h) => h,
        Err(_) => return Ok(()),
    };
    for h in hooks.iter().filter(|h| {
        h.get("config")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            == Some(url.as_str())
    }) {
        let Some(hook_id) = h.get("id").and_then(|v| v.as_i64()) else {
            continue;
        };
        let del_url = format!("https://api.github.com/repos/{owner}/{repo}/hooks/{hook_id}");
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

    /// Global lock guarding tests that mutate process-wide env vars. Cargo
    /// runs integration tests in parallel by default, so anything reading the
    /// same env keys must serialize through here.
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        ENV_LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn webhook_url_respects_base_env() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _base_url = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "https://stig.example.com/");
        assert_eq!(
            webhook_url(),
            "https://stig.example.com/api/webhooks/github"
        );
    }

    #[test]
    fn required_events_include_v1_coverage() {
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"issues"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"pull_request"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"check_suite"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"check_run"));
        assert!(REQUIRED_WEBHOOK_EVENTS.contains(&"status"));
    }

    #[test]
    fn classify_403_not_accessible_as_missing_permission() {
        let body = r#"{"message":"Resource not accessible by integration","documentation_url":"https://docs.github.com/rest/repos/webhooks#list-repository-webhooks","status":"403"}"#;
        let err = classify_github_error(
            "list the repo webhooks",
            reqwest::StatusCode::FORBIDDEN,
            body,
        );
        match err {
            ActivationError::MissingGithubPermission { action, details } => {
                assert_eq!(action, "list the repo webhooks");
                assert!(details.contains("Repository webhooks"));
                assert!(details.contains("list the repo webhooks"));
            }
            other => panic!("expected MissingGithubPermission, got {other:?}"),
        }
    }

    #[test]
    fn classify_403_without_integration_phrase_falls_back_to_github_error() {
        // Some 403s are rate-limit or org-SSO ones — don't mis-classify them
        // as a missing App permission.
        let body = r#"{"message":"API rate limit exceeded"}"#;
        let err = classify_github_error(
            "list the repo webhooks",
            reqwest::StatusCode::FORBIDDEN,
            body,
        );
        match err {
            ActivationError::GitHub(msg) => {
                assert!(msg.contains("list the repo webhooks failed"));
                assert!(msg.contains("rate limit"));
            }
            other => panic!("expected GitHub, got {other:?}"),
        }
    }

    #[test]
    fn classify_non_403_is_opaque_github_error() {
        let err = classify_github_error(
            "create the repo webhook",
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "boom",
        );
        match err {
            ActivationError::GitHub(msg) => {
                assert!(msg.contains("create the repo webhook failed"));
                assert!(msg.contains("422"));
                assert!(msg.contains("boom"));
            }
            other => panic!("expected GitHub, got {other:?}"),
        }
    }
}
