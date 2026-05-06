//! Workflow activation and deactivation hooks (issue #81).
//!
//! Spec #222 Slice 4 moved this module from stiglab to portal — portal
//! owns the GitHub side-effects of activation (label create / webhook
//! register) since it owns the credentials and the `/api/webhooks/github`
//! receiver. The workflow row's `active` flag is flipped through portal's
//! `workflow_db` writer, which targets the spine schema directly.
//!
//! Activation:
//! 1. Resolve workspace install token (via
//!    `onsager_github::api::app::mint_installation_token`).
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

use onsager_github::api::app::{list_installation_repos, InstallationToken};
use serde::Serialize;
use thiserror::Error;

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
    /// Resolution exhausted every layer (explicit env, `RAILWAY_PUBLIC_DOMAIN`,
    /// `X-Forwarded-*` headers when trust is enabled) without producing a
    /// candidate URL. Distinct from `WebhookUrlInvalid` so the dashboard can
    /// tell operators which lever to configure rather than "fix your URL".
    #[error("webhook base URL could not be determined from env or request")]
    WebhookUrlUnknown,
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

/// Stable path for the webhook receiver. Used for URL construction and,
/// more importantly, for path-suffix matching in `deregister_webhook` so
/// we can find and clean up hooks stiglab registered regardless of what
/// origin they were registered under (matters for PR-preview URL drift).
pub const WEBHOOK_PATH: &str = "/api/webhooks/github";

/// Resolve the webhook base URL (scheme + host[:port], no trailing slash)
/// from the first layer of the chain that yields a value:
///
/// 1. Explicit operator override — `STIGLAB_WEBHOOK_BASE_URL` or
///    `STIGLAB_PUBLIC_BASE_URL`. Always wins; use for tunnels and any
///    setup where auto-detect is wrong.
/// 2. Platform-injected public domain — `RAILWAY_PUBLIC_DOMAIN`
///    (Railway sets this to the hostname without a scheme;
///    we assume `https://`).
/// 3. Request-derived origin — when `STIGLAB_TRUST_FORWARDED_HEADERS=1`,
///    take `X-Forwarded-Proto` + `X-Forwarded-Host` from the incoming
///    activation request, falling back to `Host`. The gate exists
///    because these headers are spoofable by any client that can hit
///    stiglab directly without a proxy in front; trusting them in a
///    non-proxied deploy lets a hostile client redirect the workspace's
///    webhook deliveries to an attacker-controlled URL.
///
/// Returns `None` when no layer yields anything — callers map this to
/// `ActivationError::WebhookUrlUnknown`. No `localhost` default on
/// purpose: a workflow without a real webhook is silently broken, so we
/// fail loudly instead.
fn resolve_webhook_base(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Ok(explicit) = std::env::var("STIGLAB_WEBHOOK_BASE_URL")
        .or_else(|_| std::env::var("STIGLAB_PUBLIC_BASE_URL"))
    {
        let trimmed = explicit.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(domain) = std::env::var("RAILWAY_PUBLIC_DOMAIN") {
        let trimmed = domain.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            // Railway injects this as a bare hostname (no scheme); their
            // edge terminates TLS, so https is always the right scheme.
            return Some(format!("https://{trimmed}"));
        }
    }
    if trust_forwarded_headers() {
        if let Some(origin) = origin_from_headers(headers) {
            return Some(origin);
        }
    }
    None
}

/// Resolve the full webhook URL (base + path). Same semantics as
/// `resolve_webhook_base` — `None` propagates up as `WebhookUrlUnknown`.
fn resolve_webhook_url(headers: &axum::http::HeaderMap) -> Option<String> {
    resolve_webhook_base(headers).map(|base| format!("{base}{WEBHOOK_PATH}"))
}

/// Whether `STIGLAB_TRUST_FORWARDED_HEADERS` is set to a truthy value.
/// Accepts the common conventions — `1`, `true`, `yes`, `on` — all
/// case-insensitive. Anything else (including `0`, `false`, `no`, `off`,
/// and typos like `True1`) is false.
fn trust_forwarded_headers() -> bool {
    match std::env::var("STIGLAB_TRUST_FORWARDED_HEADERS")
        .ok()
        .as_deref()
    {
        Some(s) => {
            let lower = s.trim().to_ascii_lowercase();
            matches!(lower.as_str(), "1" | "true" | "yes" | "on")
        }
        None => false,
    }
}

/// Pull a public origin (scheme + host[:port]) from the incoming request
/// headers. Prefers `X-Forwarded-Proto` + `X-Forwarded-Host` (what the
/// Railway edge / Vite proxy / nginx set) over the raw `Host` header
/// since the latter is the internal hostname when behind a proxy.
///
/// Returns `None` when:
/// - neither forwarded nor `Host` header is present,
/// - the host value contains characters that would break URL structure
///   (`/`, `?`, `#`, whitespace) — a misbehaving proxy shouldn't let us
///   register a webhook under an attacker-influenced path, and
/// - `X-Forwarded-Proto` is set to anything other than `http` / `https`.
///   We don't want a proxy-supplied `javascript:` or `file:` scheme
///   leaking through classification. Missing proto falls back to `https`
///   because a trusted proxy terminates TLS and that's safer than `http`.
fn origin_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    let proto = match headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        // Some proxies set a comma-separated list; take the first.
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(s) if s.eq_ignore_ascii_case("http") => "http",
        Some(s) if s.eq_ignore_ascii_case("https") => "https",
        Some(_) => return None,
        None => "https",
    };
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())?;
    if host
        .chars()
        .any(|c| c == '/' || c == '?' || c == '#' || c.is_whitespace())
    {
        return None;
    }
    Some(format!("{proto}://{host}"))
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
    headers: &axum::http::HeaderMap,
) -> Result<i64, ActivationError> {
    let url = resolve_webhook_url(headers).ok_or(ActivationError::WebhookUrlUnknown)?;
    // Fail fast when the configured URL clearly can't be reached from
    // github.com — otherwise we'd just proxy through a 422 and the
    // dashboard would surface an opaque "github api error" instead of a
    // message the operator can act on. Also catches header-resolved
    // localhost when dev accidentally enables trust without a proxy.
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

/// Remove any repository webhook that stiglab could have registered.
///
/// Matches by path-suffix on `config.url` (`.../api/webhooks/github`)
/// rather than by exact URL. This survives origin drift — a workflow
/// activated on a Railway PR preview whose URL later changed (rebase,
/// env rename, prod cutover) still gets cleaned up instead of leaving a
/// ghost webhook delivering into the void.
///
/// The path is stable across deploys and stiglab-specific enough that
/// accidentally sweeping a non-stiglab hook with the same path is not a
/// realistic risk. No-op when nothing matches, the repo is gone, or the
/// token is rejected — this function is best-effort cleanup.
pub async fn deregister_webhook(
    token: &InstallationToken,
    owner: &str,
    repo: &str,
) -> Result<(), ActivationError> {
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
            .map(webhook_url_matches_ours)
            .unwrap_or(false)
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

/// Whether a hook's `config.url` looks like one stiglab registered —
/// i.e. its path ends with [`WEBHOOK_PATH`]. Parses through
/// `reqwest::Url` so we compare the path structurally (ignoring query
/// strings and fragments). True suffix check rather than exact equality
/// so stiglab running under a base path prefix (reverse proxy mounting
/// it at `/stiglab/…`) still matches its own hooks.
///
/// The `/api/webhooks/github` prefix is specific enough that the only
/// realistic false positive would be another service deliberately using
/// the same path, which we don't guard against — stiglab owns this path
/// by convention.
fn webhook_url_matches_ours(url: &str) -> bool {
    reqwest::Url::parse(url)
        .map(|u| u.path().trim_end_matches('/').ends_with(WEBHOOK_PATH))
        .unwrap_or(false)
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

        /// Explicitly unset a var for the scope of the guard. Needed when
        /// a test exercises a fallback layer and must guarantee the
        /// earlier layers don't resolve from ambient env.
        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
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

    /// Clear every env var the resolver reads. Use when setting up a
    /// clean slate for chain tests; each test then layers the specific
    /// vars it cares about on top.
    fn clear_resolver_env() -> [ScopedEnvVar; 4] {
        [
            ScopedEnvVar::unset("STIGLAB_WEBHOOK_BASE_URL"),
            ScopedEnvVar::unset("STIGLAB_PUBLIC_BASE_URL"),
            ScopedEnvVar::unset("RAILWAY_PUBLIC_DOMAIN"),
            ScopedEnvVar::unset("STIGLAB_TRUST_FORWARDED_HEADERS"),
        ]
    }

    fn header_map(entries: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut map = axum::http::HeaderMap::new();
        for (k, v) in entries {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn resolve_webhook_url_layer1_explicit_override_wins() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        // Set every layer; layer 1 must win.
        let _base = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "https://stig.example.com/");
        let _railway = ScopedEnvVar::set("RAILWAY_PUBLIC_DOMAIN", "ignored.up.railway.app");
        let _trust = ScopedEnvVar::set("STIGLAB_TRUST_FORWARDED_HEADERS", "1");
        let headers = header_map(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "header.example.com"),
        ]);
        assert_eq!(
            resolve_webhook_url(&headers),
            Some("https://stig.example.com/api/webhooks/github".to_string())
        );
    }

    #[test]
    fn resolve_webhook_url_layer1_public_base_url_alias() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        let _base = ScopedEnvVar::set("STIGLAB_PUBLIC_BASE_URL", "https://pub.example.com");
        assert_eq!(
            resolve_webhook_url(&axum::http::HeaderMap::new()),
            Some("https://pub.example.com/api/webhooks/github".to_string())
        );
    }

    #[test]
    fn resolve_webhook_url_layer2_railway_domain() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        let _railway = ScopedEnvVar::set("RAILWAY_PUBLIC_DOMAIN", "foo.up.railway.app");
        assert_eq!(
            resolve_webhook_url(&axum::http::HeaderMap::new()),
            Some("https://foo.up.railway.app/api/webhooks/github".to_string())
        );
    }

    #[test]
    fn resolve_webhook_url_layer3_forwarded_headers_when_trusted() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        let _trust = ScopedEnvVar::set("STIGLAB_TRUST_FORWARDED_HEADERS", "1");
        let headers = header_map(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "stig.example.com"),
        ]);
        assert_eq!(
            resolve_webhook_url(&headers),
            Some("https://stig.example.com/api/webhooks/github".to_string())
        );
    }

    #[test]
    fn resolve_webhook_url_layer3_falls_back_to_host_header() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        let _trust = ScopedEnvVar::set("STIGLAB_TRUST_FORWARDED_HEADERS", "1");
        let headers = header_map(&[("host", "stig.internal:3000")]);
        // Falls back to `https` scheme when X-Forwarded-Proto missing —
        // a trusted proxy should set it, but HTTP fallback would mean
        // no TLS which is worse than a scheme mismatch we can't verify.
        assert_eq!(
            resolve_webhook_url(&headers),
            Some("https://stig.internal:3000/api/webhooks/github".to_string())
        );
    }

    #[test]
    fn resolve_webhook_url_layer3_skipped_when_trust_disabled() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        // Trust flag unset; headers present but ignored.
        let headers = header_map(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "stig.example.com"),
        ]);
        assert_eq!(resolve_webhook_url(&headers), None);
    }

    #[test]
    fn resolve_webhook_url_none_when_chain_exhausted() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        // Even with trust enabled, an empty header map yields nothing.
        let _trust = ScopedEnvVar::set("STIGLAB_TRUST_FORWARDED_HEADERS", "1");
        assert_eq!(resolve_webhook_url(&axum::http::HeaderMap::new()), None);
    }

    #[test]
    fn trust_flag_accepts_common_truthy_values() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        for (val, expected) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("True", true),
            ("yes", true),
            ("YES", true),
            ("Yes", true),
            ("on", true),
            ("ON", true),
            // Leading/trailing whitespace is tolerated; env substitution
            // that leaves a stray newline shouldn't silently disable the
            // flag.
            (" 1 ", true),
            ("0", false),
            ("false", false),
            ("no", false),
            ("off", false),
            ("", false),
            // Typos and near-misses are disabled, not truthy.
            ("True1", false),
            ("y", false),
        ] {
            let _flag = ScopedEnvVar::set("STIGLAB_TRUST_FORWARDED_HEADERS", val);
            assert_eq!(trust_forwarded_headers(), expected, "value {val:?}");
        }
    }

    #[test]
    fn origin_from_headers_rejects_non_http_proto() {
        // A misbehaving proxy (or an attacker who can reach stiglab
        // directly) mustn't be able to smuggle a non-HTTP scheme into
        // the registered webhook URL.
        let headers = header_map(&[
            ("x-forwarded-proto", "javascript"),
            ("x-forwarded-host", "stig.example.com"),
        ]);
        assert_eq!(origin_from_headers(&headers), None);
        let headers = header_map(&[
            ("x-forwarded-proto", "file"),
            ("x-forwarded-host", "stig.example.com"),
        ]);
        assert_eq!(origin_from_headers(&headers), None);
    }

    #[test]
    fn origin_from_headers_rejects_host_with_structural_chars() {
        // Trailing slash / query / fragment / embedded whitespace in the
        // host would let a spoofed header inject path or break URL
        // structure. Refuse to resolve in those cases. (Control chars
        // like `\n` are rejected by `HeaderValue::from_str` before they
        // could reach us, so we only test the characters that are valid
        // HTTP header values but still structurally dangerous.)
        for host in [
            "stig.example.com/",
            "stig.example.com/evil",
            "stig.example.com?x=1",
            "stig.example.com#frag",
            "stig.example.com evil.com",
        ] {
            let headers = header_map(&[("x-forwarded-host", host)]);
            assert_eq!(
                origin_from_headers(&headers),
                None,
                "expected None for host {host:?}"
            );
        }
    }

    #[test]
    fn origin_from_headers_normalizes_proto_case() {
        // RFC 3986 scheme comparison is case-insensitive; Railway edge
        // occasionally emits uppercase.
        let headers = header_map(&[
            ("x-forwarded-proto", "HTTPS"),
            ("x-forwarded-host", "stig.example.com"),
        ]);
        assert_eq!(
            origin_from_headers(&headers),
            Some("https://stig.example.com".to_string())
        );
    }

    #[test]
    fn webhook_url_matches_ours_by_path_suffix() {
        // Same origin, same path → match.
        assert!(webhook_url_matches_ours(
            "https://stig.example.com/api/webhooks/github"
        ));
        // Different origin (preview URL drift) but same path → still match.
        assert!(webhook_url_matches_ours(
            "https://onsager-pr-42.up.railway.app/api/webhooks/github"
        ));
        // Trailing slash tolerated.
        assert!(webhook_url_matches_ours(
            "https://stig.example.com/api/webhooks/github/"
        ));
        // Base-path prefix (stiglab mounted under a reverse proxy path)
        // → still match. This is the case that motivates true
        // `ends_with` over exact equality.
        assert!(webhook_url_matches_ours(
            "https://proxy.example.com/stiglab/api/webhooks/github"
        ));
        // Query / fragment are stripped by URL parsing before comparison.
        assert!(webhook_url_matches_ours(
            "https://stig.example.com/api/webhooks/github?foo=1#bar"
        ));
        // Different path → no match.
        assert!(!webhook_url_matches_ours(
            "https://stig.example.com/api/webhooks/other"
        ));
        // Path prefix but not equal → no match (guards against
        // over-matching `/api/webhooks/github-extra`).
        assert!(!webhook_url_matches_ours(
            "https://stig.example.com/api/webhooks/github-extra"
        ));
        // Near-miss segment boundary — `/xapi/webhooks/github` must
        // NOT match, because the suffix starts mid-segment. The leading
        // `/` in WEBHOOK_PATH keeps this honest.
        assert!(!webhook_url_matches_ours(
            "https://stig.example.com/xapi/webhooks/github"
        ));
        // Unparseable → no match (best-effort cleanup never errors).
        assert!(!webhook_url_matches_ours("not a url"));
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
        let _clear = clear_resolver_env();
        let _base = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "http://localhost:3000");
        let token = InstallationToken {
            token: "t".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        let err = ensure_webhook_registered(
            &token,
            "acme",
            "widgets",
            None,
            &axum::http::HeaderMap::new(),
        )
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
        let _clear = clear_resolver_env();
        let _base = ScopedEnvVar::set("STIGLAB_WEBHOOK_BASE_URL", "not a url");
        let token = InstallationToken {
            token: "t".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        let err = ensure_webhook_registered(
            &token,
            "acme",
            "widgets",
            None,
            &axum::http::HeaderMap::new(),
        )
        .await
        .expect_err("expected invalid-url rejection");
        match err {
            ActivationError::WebhookUrlInvalid { url } => {
                assert!(url.starts_with("not a url"), "got {url}");
            }
            other => panic!("expected WebhookUrlInvalid, got {other:?}"),
        }
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn ensure_webhook_registered_errors_when_chain_unresolved() {
        let _env_guard = env_lock().lock().expect("env lock poisoned");
        let _clear = clear_resolver_env();
        let token = InstallationToken {
            token: "t".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        let err = ensure_webhook_registered(
            &token,
            "acme",
            "widgets",
            None,
            &axum::http::HeaderMap::new(),
        )
        .await
        .expect_err("expected unresolved-chain rejection");
        assert!(
            matches!(err, ActivationError::WebhookUrlUnknown),
            "got {err:?}"
        );
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
