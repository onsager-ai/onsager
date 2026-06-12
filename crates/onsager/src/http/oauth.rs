//! GitHub OAuth login (M2 #624). Ported from the legacy portal's auth
//! handlers, cut to the standard dance: redirect with a CSRF state
//! cookie → callback verifies state, exchanges the code, upserts the
//! user, mints the same session cookie dev-login mints. The legacy
//! cross-environment SSO machinery did not port (no consumer).

use axum::extract::{Query, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{User, create_auth_session, parse_cookie, session_cookie, upsert_user};
use crate::crypto::random_token;
use crate::state::AppState;

const STATE_COOKIE: &str = "onsager_oauth_state";

fn callback_url(state: &AppState) -> String {
    let origin = state
        .config
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", state.config.bind));
    format!("{origin}/api/auth/github/callback")
}

/// `GET /api/auth/github` — start the dance.
pub async fn start(State(state): State<AppState>) -> Response {
    let Some(client_id) = state.config.github_client_id.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub OAuth not configured",
        )
            .into_response();
    };
    let csrf = random_token();
    let url =
        onsager_github::api::oauth::github_authorize_url(client_id, &callback_url(&state), &csrf);
    (
        [(
            header::SET_COOKIE,
            format!("{STATE_COOKIE}={csrf}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600"),
        )],
        Redirect::temporary(&url),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

/// `GET /api/auth/github/callback`.
pub async fn callback(
    State(state): State<AppState>,
    parts: Parts,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let (Some(client_id), Some(client_secret)) = (
        state.config.github_client_id.as_deref(),
        state.config.github_client_secret.as_deref(),
    ) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub OAuth not configured",
        )
            .into_response();
    };
    let cookie_state = parts
        .headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| parse_cookie(h, STATE_COOKIE));
    if cookie_state != Some(q.state.as_str()) {
        return (StatusCode::UNAUTHORIZED, "oauth state mismatch").into_response();
    }

    let token =
        match onsager_github::api::oauth::exchange_code(client_id, client_secret, &q.code).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("oauth exchange failed: {e}");
                return (StatusCode::BAD_GATEWAY, "oauth exchange failed").into_response();
            }
        };
    let gh_user = match onsager_github::api::oauth::get_oauth_user(&token.access_token).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("oauth user fetch failed: {e}");
            return (StatusCode::BAD_GATEWAY, "oauth user fetch failed").into_response();
        }
    };

    let user = User {
        id: Uuid::new_v4().to_string(),
        github_id: gh_user.id,
        github_login: gh_user.login.clone(),
        github_name: gh_user.name.clone(),
        github_avatar_url: gh_user.avatar_url.clone(),
    };
    if let Err(e) = upsert_user(&state.pool, &user).await {
        tracing::error!("oauth user upsert failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // The upsert keys on github_id; re-resolve the canonical row id.
    let user = match crate::auth::get_user_by_github_id(&state.pool, gh_user.id).await {
        Ok(Some(u)) => u,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let session_token = random_token();
    if let Err(e) = create_auth_session(
        &state.pool,
        &session_token,
        &user.id,
        Utc::now() + chrono::Duration::days(30),
    )
    .await
    {
        tracing::error!("oauth session mint failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    (
        [
            (header::SET_COOKIE, session_cookie(&state, &session_token)),
            (
                header::SET_COOKIE,
                format!("{STATE_COOKIE}=; Path=/; HttpOnly; Max-Age=0"),
            ),
        ],
        Redirect::temporary("/"),
    )
        .into_response()
}
