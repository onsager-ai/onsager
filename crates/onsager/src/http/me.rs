//! `/api/auth/*` session-introspection routes (dev-login itself lives
//! in [`crate::auth`]).

use axum::Json;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::auth::{AuthUser, parse_cookie};
use crate::state::AppState;

/// Which login buttons the dashboard renders. GitHub OAuth returns in
/// M2 with the GitHub port; until then only dev-login can exist.
pub async fn auth_providers(State(state): State<AppState>) -> Response {
    Json(serde_json::json!({
        "github": state.config.github_client_id.is_some()
            && state.config.github_client_secret.is_some(),
        "dev": state.config.dev_login_enabled(),
    }))
    .into_response()
}

pub async fn get_me(user: AuthUser) -> Response {
    Json(serde_json::json!({
        "session_kind": user.session_kind,
        "user": {
            "id": user.user_id,
            "github_login": user.github_login,
            "github_name": user.github_name,
            "github_avatar_url": user.github_avatar_url,
        },
    }))
    .into_response()
}

pub async fn logout(State(state): State<AppState>, parts: Parts) -> Response {
    if let Some(session_id) = parts
        .headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| parse_cookie(h, "onsager_session"))
        && let Err(e) = crate::auth::delete_auth_session(&state.pool, session_id).await
    {
        tracing::warn!("logout: failed to delete session: {e}");
    }
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            "onsager_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}
