use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::core::User;

use crate::server::auth::{
    self, exchange_code, generate_session_token, generate_state, get_github_user,
    github_authorize_url, AuthUser,
};
use crate::server::db;
use crate::server::state::AppState;

#[derive(serde::Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

/// GET /api/auth/github — Redirect to GitHub OAuth
pub async fn github_login(State(state): State<AppState>) -> impl IntoResponse {
    let config = &state.config;

    let Some(ref client_id) = config.github_client_id else {
        return (StatusCode::NOT_FOUND, "auth not configured").into_response();
    };

    let callback_url = build_callback_url(config);
    let csrf_state = generate_state();

    let url = github_authorize_url(client_id, &callback_url, &csrf_state);

    // Set CSRF state in cookie
    let sec = secure_attr(config);
    let cookie = format!(
        "stiglab_oauth_state={csrf_state}; Path=/; HttpOnly; SameSite=Lax; Max-Age=600{sec}"
    );

    ([(header::SET_COOKIE, cookie)], Redirect::temporary(&url)).into_response()
}

/// GET /api/auth/github/callback — Handle OAuth callback
pub async fn github_callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let config = &state.config;

    let (Some(ref client_id), Some(ref client_secret)) =
        (&config.github_client_id, &config.github_client_secret)
    else {
        return (StatusCode::NOT_FOUND, "auth not configured").into_response();
    };

    // Verify CSRF state
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let stored_state = auth::parse_cookie(cookie_header, "stiglab_oauth_state");

    if stored_state != Some(params.state.as_str()) {
        return (StatusCode::BAD_REQUEST, "invalid OAuth state").into_response();
    }

    // Exchange code for token
    let token = match exchange_code(client_id, client_secret, &params.code).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("GitHub token exchange failed: {e}");
            return (StatusCode::BAD_GATEWAY, "GitHub authentication failed").into_response();
        }
    };

    // Fetch GitHub user profile
    let gh_user = match get_github_user(&token.access_token).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("GitHub user API failed: {e}");
            return (StatusCode::BAD_GATEWAY, "Failed to fetch GitHub profile").into_response();
        }
    };

    // Upsert user in DB
    let existing = db::get_user_by_github_id(&state.db, gh_user.id)
        .await
        .ok()
        .flatten();
    let user_id = existing
        .as_ref()
        .map(|u| u.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let user = User {
        id: user_id.clone(),
        github_id: gh_user.id,
        github_login: gh_user.login,
        github_name: gh_user.name,
        github_avatar_url: gh_user.avatar_url,
        created_at: existing.map(|u| u.created_at).unwrap_or_else(Utc::now),
        updated_at: Utc::now(),
    };

    if let Err(e) = db::upsert_user(&state.db, &user).await {
        tracing::error!("failed to upsert user: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    // Create auth session
    let session_token = generate_session_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);

    if let Err(e) = db::create_auth_session(&state.db, &session_token, &user_id, expires_at).await {
        tracing::error!("failed to create auth session: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    tracing::info!("user logged in: {} ({})", user.github_login, user_id);

    // Set session cookie and clear CSRF cookie
    let sec = secure_attr(config);
    let session_cookie = format!(
        "stiglab_session={session_token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{sec}"
    );
    let clear_state_cookie =
        format!("stiglab_oauth_state=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, session_cookie)
        .header(header::SET_COOKIE, clear_state_cookie)
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

/// GET /api/auth/me — Return current authenticated user
pub async fn me(State(state): State<AppState>, auth_user: AuthUser) -> impl IntoResponse {
    // Return auth_enabled flag so frontend knows whether to show login
    Json(serde_json::json!({
        "user": {
            "id": auth_user.user_id,
            "github_login": auth_user.github_login,
            "github_name": auth_user.github_name,
            "github_avatar_url": auth_user.github_avatar_url,
        },
        "auth_enabled": state.config.auth_enabled(),
    }))
}

/// POST /api/auth/logout — Delete auth session and clear cookie
pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(session_id) = auth::parse_cookie(cookie_header, "stiglab_session") {
        let _ = db::delete_auth_session(&state.db, session_id).await;
    }

    let sec = secure_attr(&state.config);
    let clear_cookie = format!("stiglab_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{sec}");

    (
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
}

fn build_callback_url(config: &crate::server::config::ServerConfig) -> String {
    if let Some(ref public_url) = config.public_url {
        format!("{public_url}/api/auth/github/callback")
    } else {
        format!("http://localhost:{}/api/auth/github/callback", config.port)
    }
}

/// Returns "; Secure" if the public URL uses HTTPS, empty string otherwise.
fn secure_attr(config: &crate::server::config::ServerConfig) -> &'static str {
    if config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    }
}
