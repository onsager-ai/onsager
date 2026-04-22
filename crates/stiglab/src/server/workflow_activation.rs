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
    /// generic 502 we use for opaque upstream failures. `upstream` carries
    /// the raw `status: body` pair so operator logs keep the breadcrumb the
    /// 502 path previously had.
    #[error("github app install is missing permission for {action}")]
    MissingGithubPermission {
        action: String,
        permission: &'static str,
        details: String,
        upstream: String,
    },
    #[error("github api error: {0}")]
    GitHub(String),
    /// The configured webhook URL points at a loopback/private host that
    /// github.com cannot reach, so registering the hook would fail with a
    /// 422. Surfacing this as a typed error lets the dashboard show an
    /// operator-actionable message instead of the opaque GitHub 422.
    #[error("webhook URL {url} is not reachable from GitHub")]
    WebhookUrlNotReachable { url: String },
    /// The configured webhook URL isn't a valid absolute URL with a host —
    /// usually a typo in `STIGLAB_WEBHOOK_BASE_URL`. Distinct from
    /// `WebhookUrlNotReachable` so the dashboard can point the operator at
    /// the real problem (fix the URL) vs. the other real problem (use a
    /// public host).
    #[error("webhook URL {url} is not a valid absolute URL")]
    WebhookUrlInvalid { url: String },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Why a webhook URL was rejected. Lets the caller pick the right
/// user-facing error variant.
#[derive(Debug, PartialEq, Eq)]
enum WebhookUrlReject {
    /// Parse error, hostless URL, or otherwise not a usable absolute URL.
    Invalid,
    /// Parsed fine, but the host is clearly unreachable from github.com
    /// (loopback, private, link-local, `localhost`).
    NotReachable,
}

/// Classify a non-2xx response from the GitHub REST API. Returns a
/// `MissingGithubPermission` when the status + body match the signature of a
/// missing App permission (so the caller surfaces a dashboard-actionable 4xx);
/// otherwise falls back to the opaque `GitHub` variant. `permission` is the
/// human-readable name the caller knows the endpoint needs (e.g.
/// `"Repository webhooks: Read & write"` or `"Issues: Read & write"`) — GitHub
/// returns the same opaque 403 for any missing App permission, so we can't
/// derive it from the response.
fn classify_github_error(
    action: &str,
    permission: &'static str,
    status: reqwest::StatusCode,
    body: &str,
) -> ActivationError {
    if status.as_u16() == 403 && body.contains("Resource not accessible by integration") {
        return ActivationError::MissingGithubPermission {
            action: action.to_string(),
            permission,
            details: format!(
                "GitHub App install is missing the '{permission}' permission required to {action}. Update the App's permissions on GitHub and accept the new permission request on the installation, then retry."
            ),
            upstream: format!("{status}: {body}"),
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

/// Classify a webhook URL: usable, invalid, or parseable-but-unreachable.
/// We reject loopback, unspecified, RFC1918-style private, and link-local
/// addresses (for both IPv4 and IPv6) plus the `localhost` hostname (and
/// its reserved subdomains per RFC 6761, tolerant of a trailing
/// absolute-DNS dot). Everything else — including DNS names we can't
/// resolve here — is optimistically accepted; GitHub itself is the final
/// arbiter.
fn classify_webhook_url(url: &str) -> Result<(), WebhookUrlReject> {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return Err(WebhookUrlReject::Invalid);
    };
    let Some(host) = parsed.host_str() else {
        return Err(WebhookUrlReject::Invalid);
    };
    // `host_str()` returns IPv6 literals wrapped in `[...]`; strip the
    // brackets before handing the string to `IpAddr::from_str`.
    let host_trimmed = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = host_trimmed.parse::<std::net::IpAddr>() {
        if ip.is_loopback() || ip.is_unspecified() {
            return Err(WebhookUrlReject::NotReachable);
        }
        match ip {
            std::net::IpAddr::V4(v4) => {
                if v4.is_private() || v4.is_link_local() {
                    return Err(WebhookUrlReject::NotReachable);
                }
            }
            std::net::IpAddr::V6(v6) => {
                // `fc00::/7` (unique local) and `fe80::/10` (link-local
                // unicast) are also unreachable from GitHub's servers.
                let seg0 = v6.segments()[0];
                let is_unique_local = (seg0 & 0xfe00) == 0xfc00;
                let is_link_local = (seg0 & 0xffc0) == 0xfe80;
                if is_unique_local || is_link_local {
                    return Err(WebhookUrlReject::NotReachable);
                }
            }
        }
        return Ok(());
    }
    // DNS name. Normalize trailing dots (absolute DNS form) before the
    // `localhost` family comparison.
    let host_lower = host.to_ascii_lowercase();
    let host_normalized = host_lower.trim_end_matches('.');
    if host_normalized == "localhost" || host_normalized.ends_with(".localhost") {
        return Err(WebhookUrlReject::NotReachable);
    }
    Ok(())
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
        return Err(classify_github_error(
            "look up repo labels",
            "Issues: Read & write",
            status,
            &body,
        ));
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
        "Issues: Read & write",
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
    // Fail fast when the configured URL clearly can't be reached from
    // github.com — otherwise we'd just proxy through a 422 and the
    // dashboard would surface an opaque "github api error" instead of a
    // message the operator can act on.
    match classify_webhook_url(&url) {
        Ok(()) => {}
        Err(WebhookUrlReject::Invalid) => return Err(ActivationError::WebhookUrlInvalid { url }),
        Err(WebhookUrlReject::NotReachable) => {
            return Err(ActivationError::WebhookUrlNotReachable { url })
        }
    }
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
            "Repository webhooks: Read & write",
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
                "Repository webhooks: Read & write",
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
            "Repository webhooks: Read & write",
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
            "Repository webhooks: Read & write",
            reqwest::StatusCode::FORBIDDEN,
            body,
        );
        match err {
            ActivationError::MissingGithubPermission {
                action,
                permission,
                details,
                upstream,
            } => {
                assert_eq!(action, "list the repo webhooks");
                assert_eq!(permission, "Repository webhooks: Read & write");
                assert!(details.contains("Repository webhooks: Read & write"));
                assert!(details.contains("list the repo webhooks"));
                // No hard wraps embedded in the user-visible string.
                assert!(!details.contains("  "));
                // Upstream breadcrumb preserved for operator logs.
                assert!(upstream.contains("403"));
                assert!(upstream.contains("Resource not accessible by integration"));
            }
            other => panic!("expected MissingGithubPermission, got {other:?}"),
        }
    }

    #[test]
    fn classify_403_label_path_mentions_issues_permission() {
        // The same opaque 403 on a label endpoint must point at the Issues
        // permission — not the webhooks one — since the caller knows which
        // permission the endpoint needs.
        let body = r#"{"message":"Resource not accessible by integration"}"#;
        let err = classify_github_error(
            "look up repo labels",
            "Issues: Read & write",
            reqwest::StatusCode::FORBIDDEN,
            body,
        );
        match err {
            ActivationError::MissingGithubPermission {
                permission,
                details,
                ..
            } => {
                assert_eq!(permission, "Issues: Read & write");
                assert!(details.contains("Issues: Read & write"));
                assert!(!details.contains("Repository webhooks"));
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
            "Repository webhooks: Read & write",
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
    fn classify_accepts_public_dns_and_ip_hosts() {
        assert_eq!(
            classify_webhook_url("https://stig.example.com/api/webhooks/github"),
            Ok(())
        );
        assert_eq!(
            classify_webhook_url("https://stig.example.com:8443/api/webhooks/github"),
            Ok(())
        );
        assert_eq!(
            classify_webhook_url("https://8.8.8.8/api/webhooks/github"),
            Ok(())
        );
        // Http is fine too — GitHub accepts either scheme on webhooks.
        assert_eq!(
            classify_webhook_url("http://stig.example.com/api/webhooks/github"),
            Ok(())
        );
    }

    #[test]
    fn classify_rejects_localhost_family_as_not_reachable() {
        for url in [
            "http://localhost:3000/api/webhooks/github",
            "http://LOCALHOST/api/webhooks/github",
            "http://foo.localhost/api/webhooks/github",
            // Trailing dot (absolute DNS form) — still localhost per RFC 6761.
            "http://localhost./api/webhooks/github",
            "http://foo.localhost./api/webhooks/github",
            "http://127.0.0.1:3000/api/webhooks/github",
            "http://127.1.2.3/api/webhooks/github",
            "http://[::1]/api/webhooks/github",
            "http://0.0.0.0:3000/api/webhooks/github",
        ] {
            assert_eq!(
                classify_webhook_url(url),
                Err(WebhookUrlReject::NotReachable),
                "expected NotReachable for {url}"
            );
        }
    }

    #[test]
    fn classify_rejects_private_and_link_local_v4() {
        for url in [
            "http://10.0.0.5/api/webhooks/github",
            "http://192.168.1.20/api/webhooks/github",
            "http://172.16.0.1/api/webhooks/github",
            "http://169.254.169.254/api/webhooks/github",
        ] {
            assert_eq!(
                classify_webhook_url(url),
                Err(WebhookUrlReject::NotReachable),
                "expected NotReachable for {url}"
            );
        }
    }

    #[test]
    fn classify_rejects_private_and_link_local_v6() {
        for url in [
            // Unique local (fc00::/7).
            "http://[fc00::1]/api/webhooks/github",
            "http://[fd12:3456::1]/api/webhooks/github",
            // Unicast link-local (fe80::/10).
            "http://[fe80::1]/api/webhooks/github",
            "http://[febf::1]/api/webhooks/github",
        ] {
            assert_eq!(
                classify_webhook_url(url),
                Err(WebhookUrlReject::NotReachable),
                "expected NotReachable for {url}"
            );
        }
    }

    #[test]
    fn classify_rejects_unparseable_as_invalid() {
        assert_eq!(
            classify_webhook_url("not a url"),
            Err(WebhookUrlReject::Invalid)
        );
        // Parses as a URL but has no host.
        assert_eq!(
            classify_webhook_url("data:text/plain,hello"),
            Err(WebhookUrlReject::Invalid)
        );
    }

    // Held across an await, but that's the whole point of `env_lock` — it
    // serializes tests that mutate process-wide env. Dropping the guard
    // before the await would let a parallel test race with us.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn ensure_webhook_registered_rejects_explicit_localhost_base_url() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _base = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "http://localhost:3000");
        let token = InstallationToken {
            token: "t".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        let err = ensure_webhook_registered(&token, "acme", "widgets", None)
            .await
            .expect_err("expected localhost rejection");
        match err {
            ActivationError::WebhookUrlNotReachable { url } => {
                assert!(url.starts_with("http://localhost:3000/"), "got {url}");
            }
            other => panic!("expected WebhookUrlNotReachable, got {other:?}"),
        }
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn ensure_webhook_registered_rejects_invalid_base_url() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _base = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "not a url");
        let token = InstallationToken {
            token: "t".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        let err = ensure_webhook_registered(&token, "acme", "widgets", None)
            .await
            .expect_err("expected invalid-url rejection");
        match err {
            ActivationError::WebhookUrlInvalid { url } => {
                assert!(url.starts_with("not a url"), "got {url}");
            }
            other => panic!("expected WebhookUrlInvalid, got {other:?}"),
        }
    }

    #[test]
    fn classify_non_403_is_opaque_github_error() {
        let err = classify_github_error(
            "create the repo webhook",
            "Repository webhooks: Read & write",
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
