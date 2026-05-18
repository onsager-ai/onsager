//! GitHub App install-flow + discovery routes (spec #222 Slice 3b —
//! moved from stiglab).
//!
//! Endpoints:
//! - `GET /api/github-app/config` — feature-detection (used by the
//!   dashboard to decide whether to show "Install via GitHub App" vs
//!   manual-entry).
//! - `GET /api/github-app/install-start?workspace_id=...` — redirect the
//!   user to GitHub's App installation page, carrying the target
//!   workspace in the OAuth `state` param.
//! - `GET /api/github-app/callback?installation_id=N&state=...` — the
//!   App's Setup URL on GitHub. Verifies the state cookie, mints an
//!   App JWT to look up the install's account, persists the
//!   installation row, and redirects the browser back to the dashboard.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use onsager_github::api::app as gh_app;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{AuthUser, generate_state, parse_cookie};
use crate::handlers::installations::require_workspace_access;
use crate::installation::GitHubAppInstallation;
use crate::installation_db;
use crate::state::AppState;

/// OAuth `state`-CSRF cookie name. Kept as the legacy `stiglab_*`
/// value byte-for-byte so any install flow already in progress when
/// this slice deploys (cookie set by the stiglab handler before the
/// switch, callback hitting the portal handler after) round-trips
/// without a 400. The cookie has a 10-minute `Max-Age` so the legacy
/// name retires naturally; a follow-up can rename to
/// `onsager_github_app_state` once an operational window passes.
const STATE_COOKIE: &str = "stiglab_github_app_state";

/// Post-install return-target cookie (spec #402). The FTUE binding flow
/// passes `?return_to=/chat?…&bind=continue` on install-start; the
/// portal stashes it here, GitHub redirects back to the callback, and
/// the callback reads it to send the browser to the binding dialog
/// instead of the default `/workspaces?…` landing. Validated to be a
/// same-origin path; anything else is dropped silently.
const RETURN_TO_COOKIE: &str = "onsager_github_app_return_to";

/// Validate that a `return_to` value is a safe same-origin path. We
/// require an explicit leading `/` (so callers can't pass a scheme +
/// host) and forbid the `//` and `/\` prefixes that browsers treat as
/// protocol-relative. Length is capped to keep the cookie payload
/// reasonable.
fn is_safe_return_to(value: &str) -> bool {
    if value.is_empty() || value.len() > 512 {
        return false;
    }
    if !value.starts_with('/') {
        return false;
    }
    if value.starts_with("//") || value.starts_with("/\\") {
        return false;
    }
    true
}

/// GET /api/github-app/config — Tiny discovery endpoint so the
/// dashboard can decide whether to render the "Install via GitHub App"
/// button or fall back to the manual-entry form.
pub async fn config() -> Response {
    let enabled = gh_app::AppConfig::from_env().is_some();
    let slug = std::env::var("GITHUB_APP_SLUG").ok();
    Json(serde_json::json!({ "enabled": enabled, "slug": slug })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct InstallStartQuery {
    pub workspace_id: String,
    /// Optional same-origin path the callback should redirect to instead
    /// of `/workspaces?...` (spec #402 binding-flow resume). Stashed in
    /// a short-lived cookie at install-start, consumed at the callback.
    /// Anything that isn't a safe same-origin path is dropped silently.
    pub return_to: Option<String>,
}

/// GET /api/github-app/install-start?workspace_id=...&return_to=...
pub async fn install_start(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<InstallStartQuery>,
) -> Response {
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &query.workspace_id).await {
        return r;
    }

    let Some(cfg) = gh_app::AppConfig::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "GitHub App is not configured on this server"
            })),
        )
            .into_response();
    };

    // state = "{workspace_id}.{csrf_random}" — cookie stores the same
    // thing so the callback can verify it came from this browser
    // session.
    let csrf = generate_state();
    let state_param = format!("{}.{}", query.workspace_id, csrf);
    let sec = if state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    };
    let state_cookie =
        format!("{STATE_COOKIE}={state_param}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}");

    let return_cookie = query.return_to.as_deref().and_then(|rt| {
        if is_safe_return_to(rt) {
            // Base64-encode the path so cookie parsing doesn't choke
            // on `=`, `;`, or `,` characters in the query string. The
            // callback decodes before redirecting; URL_SAFE_NO_PAD
            // produces a cookie-safe alphabet (the same engine `auth.rs`
            // already uses for opaque session ids and SSO codes).
            let encoded = URL_SAFE_NO_PAD.encode(rt.as_bytes());
            Some(format!(
                "{RETURN_TO_COOKIE}={encoded}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}"
            ))
        } else {
            tracing::warn!(
                "github_app install_start dropping unsafe return_to (len={})",
                rt.len()
            );
            None
        }
    });

    let url = format!(
        "https://github.com/apps/{slug}/installations/new?state={state_param}",
        slug = cfg.slug,
    );
    let mut builder = axum::response::Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header(header::LOCATION, url)
        .header(header::SET_COOKIE, state_cookie);
    if let Some(cookie) = return_cookie {
        builder = builder.header(header::SET_COOKIE, cookie);
    }
    builder
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct InstallCallbackQuery {
    pub installation_id: i64,
    pub setup_action: Option<String>,
    pub state: Option<String>,
}

/// GET /api/github-app/callback?installation_id=N&setup_action=install&state=...
///
/// GitHub redirects here after the user installs the App (this path is
/// the App's Setup URL on GitHub). We verify the state cookie, mint an
/// App JWT to look up the install's account, persist the installation
/// row under the originating workspace, and redirect the browser back
/// to `/workspaces?github_app_linked={id}` so `WorkspaceCard`'s
/// useEffect can invalidate the installations query without a manual
/// refresh.
pub async fn install_callback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: axum::http::HeaderMap,
    Query(query): Query<InstallCallbackQuery>,
) -> Response {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookie_state = parse_cookie(cookie_header, STATE_COOKIE);
    let query_state = query.state.as_deref().unwrap_or_default();
    if cookie_state != Some(query_state) || query_state.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
    }
    let workspace_id = match query_state.split_once('.') {
        Some((t, _)) if !t.is_empty() => t.to_string(),
        _ => return (StatusCode::BAD_REQUEST, "malformed state").into_response(),
    };
    if let Err(r) = require_workspace_access(&state.pool, &auth_user, &workspace_id).await {
        return r;
    }

    let Some(cfg) = gh_app::AppConfig::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub App is not configured on this server",
        )
            .into_response();
    };

    let jwt = match gh_app::mint_app_jwt(&cfg) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("mint_app_jwt failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "GitHub App auth failed").into_response();
        }
    };
    let info = match gh_app::get_installation(&jwt, query.installation_id).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("get_installation {} failed: {e}", query.installation_id);
            return (StatusCode::BAD_GATEWAY, "GitHub installation lookup failed").into_response();
        }
    };

    // Idempotency: if the user re-runs the install flow (or GitHub
    // redelivers the callback), we must not blind-insert — the numeric
    // `install_id` is UNIQUE. Pre-check and either treat as a no-op
    // (same workspace) or refuse with 409 (different workspace).
    match installation_db::get_installation_by_install_id(&state.pool, query.installation_id).await
    {
        Ok(Some(existing)) if existing.workspace_id == workspace_id => {
            tracing::info!(
                "GitHub App installation {} already linked to workspace {}; treating callback as idempotent",
                query.installation_id,
                workspace_id
            );
        }
        Ok(Some(existing)) => {
            tracing::warn!(
                "GitHub App installation {} is already linked to workspace {}; requested workspace {}",
                query.installation_id,
                existing.workspace_id,
                workspace_id
            );
            return (
                StatusCode::CONFLICT,
                "GitHub installation is already linked to a different workspace",
            )
                .into_response();
        }
        Ok(None) => {
            let install = GitHubAppInstallation {
                id: Uuid::new_v4().to_string(),
                workspace_id: workspace_id.clone(),
                install_id: query.installation_id,
                account_login: info.account_login,
                account_type: info.account_kind.into(),
                created_at: Utc::now(),
            };
            // No webhook secret here — the App-managed shared secret
            // is a server env var (portal reads it); per-install
            // override remains the manual endpoint's job.
            if let Err(e) = installation_db::insert_installation(&state.pool, &install, None).await
            {
                tracing::error!("insert_installation failed: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "GitHub installation link failed",
                )
                    .into_response();
            }
        }
        Err(e) => {
            tracing::error!("install_id lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub installation link could not be verified",
            )
                .into_response();
        }
    }

    let sec = if state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    };
    let clear_state = format!("{STATE_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");
    let clear_return =
        format!("{RETURN_TO_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");

    // Spec #402: honour an optional `return_to` cookie set at
    // install-start. The binding flow uses this to send the user back
    // to /chat with `bind=continue`; without it, we fall back to the
    // legacy `/workspaces?github_app_linked=…` landing the workspace
    // card already handles. Append the install-success params either
    // way so React Query caches can invalidate on the destination page.
    let return_to_raw = parse_cookie(cookie_header, RETURN_TO_COOKIE);
    let return_to_decoded = return_to_raw
        .and_then(|raw| URL_SAFE_NO_PAD.decode(raw).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| is_safe_return_to(decoded));

    let location = match return_to_decoded {
        Some(path) => {
            let separator = if path.contains('?') { '&' } else { '?' };
            format!(
                "{path}{separator}github_app_linked={}&workspace_id={}",
                query.installation_id, workspace_id
            )
        }
        None => format!(
            "/workspaces?github_app_linked={}&workspace_id={}",
            query.installation_id, workspace_id
        ),
    };

    axum::response::Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, clear_state)
        .header(header::SET_COOKIE, clear_return)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_accepts_same_origin_paths() {
        assert!(is_safe_return_to("/chat"));
        assert!(is_safe_return_to("/chat?bind=continue"));
        assert!(is_safe_return_to(
            "/chat?draft=abc&bind=continue&workspace_id=ws_1"
        ));
        assert!(is_safe_return_to("/workspaces/acme/workflows"));
    }

    #[test]
    fn return_to_rejects_open_redirects() {
        // Protocol-relative — would let an attacker pivot to a third
        // party origin once the browser fills in the scheme.
        assert!(!is_safe_return_to("//evil.example.com/x"));
        // Backslash variant the URL parser may normalize to `//`.
        assert!(!is_safe_return_to("/\\evil.example.com/x"));
        // Absolute URL — explicitly forbidden.
        assert!(!is_safe_return_to("https://evil.example.com/x"));
        assert!(!is_safe_return_to("javascript:alert(1)"));
        // Empty / oversized payloads.
        assert!(!is_safe_return_to(""));
        let huge = "/".to_string() + &"a".repeat(1024);
        assert!(!is_safe_return_to(&huge));
    }

    #[test]
    fn return_to_base64_round_trips() {
        let original = "/chat?draft=abc&bind=continue&workspace_id=ws_1";
        let encoded = URL_SAFE_NO_PAD.encode(original.as_bytes());
        // The encoded form must not contain cookie-breaking chars.
        assert!(!encoded.contains(';'));
        assert!(!encoded.contains(','));
        assert!(!encoded.contains('='));
        let decoded_bytes = URL_SAFE_NO_PAD.decode(encoded).unwrap();
        assert_eq!(std::str::from_utf8(&decoded_bytes).unwrap(), original);
    }
}
