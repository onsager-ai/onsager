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

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use chrono::Utc;
use onsager_github::api::app as gh_app;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{generate_state, parse_cookie, AuthUser};
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
}

/// GET /api/github-app/install-start?workspace_id=...
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
    let cookie =
        format!("{STATE_COOKIE}={state_param}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}");
    let url = format!(
        "https://github.com/apps/{slug}/installations/new?state={state_param}",
        slug = cfg.slug,
    );
    ([(header::SET_COOKIE, cookie)], Redirect::temporary(&url)).into_response()
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
    let clear = format!("{STATE_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");
    let location = format!(
        "/workspaces?github_app_linked={}&workspace_id={}",
        query.installation_id, workspace_id
    );

    axum::response::Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, clear)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}
