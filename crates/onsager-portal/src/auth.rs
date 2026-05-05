//! Auth primitives for portal-served `/api/auth/*` routes.
//!
//! Owns the `users`, `auth_sessions`, and `sso_exchange_codes` tables
//! (portal migrations 002–004). Portal mints session cookies on the
//! OAuth callback and the SSO finish path; downstream stiglab routes
//! still validate the cookie out-of-band against the same tables —
//! one DB, one writer (portal), readers wherever.
//!
//! Slice 5 of spec #222 introduces this module. Slice 2 will follow
//! up with PAT verification + credential encryption (still in stiglab
//! today).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ring::rand::{SecureRandom, SystemRandom};

use crate::auth_db;
use crate::state::AppState;

// ── GitHub OAuth (re-exports) ──
//
// Implementations live in `onsager_github::api::oauth`. Re-exported here
// so route handlers don't have to import two namespaces.

pub use onsager_github::api::oauth::{
    exchange_code, github_authorize_url, GithubOAuthUser as GithubUser, GithubTokenResponse,
};

/// Fetch the GitHub user behind an OAuth access token.
pub async fn get_github_user(access_token: &str) -> anyhow::Result<GithubUser> {
    Ok(onsager_github::api::oauth::get_oauth_user(access_token).await?)
}

/// Generate a random session token (hex-encoded, 32 bytes of randomness).
pub fn generate_session_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .expect("failed to generate random bytes");
    hex::encode(bytes)
}

/// Generate a random CSRF state parameter.
pub fn generate_state() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes)
        .expect("failed to generate random bytes");
    hex::encode(bytes)
}

// ── Auth Extractor ──

/// How the user behind a request was minted. The dashboard renders a
/// persistent dev-mode banner when this is `Dev` (issue #193).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// Real GitHub OAuth user.
    Github,
    /// Local-only dev user seeded by `seed_dev_user_and_workspace` and
    /// minted via `/api/auth/dev-login`. Only reachable in debug builds.
    Dev,
}

/// Negative `github_id` is the type-level signal for a dev-seeded user
/// (real GitHub user IDs are always positive). Centralized here so the
/// seeder, the auth extractor, and `/api/auth/me` agree on the rule.
pub fn session_kind_for_github_id(github_id: i64) -> SessionKind {
    if github_id < 0 {
        SessionKind::Dev
    } else {
        SessionKind::Github
    }
}

/// Authenticated user extracted from the `stiglab_session` cookie.
///
/// Portal's extractor is cookie-only — PAT bearer auth is still served
/// by stiglab (it owns the `user_pats` table until Slice 2). Portal
/// doesn't yet host any route that accepts a PAT, so the slim shape is
/// sufficient.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub github_login: String,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub session_kind: SessionKind,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session_id = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|cookie_header| parse_cookie(cookie_header, "stiglab_session"))
            .map(|s| s.to_string());

        let Some(session_id) = session_id else {
            return Err((StatusCode::UNAUTHORIZED, "not authenticated").into_response());
        };

        match auth_db::get_auth_session(&state.pool, &session_id).await {
            Ok(Some(auth_session)) => {
                let session_kind = session_kind_for_github_id(auth_session.user.github_id);
                Ok(AuthUser {
                    user_id: auth_session.user_id,
                    github_login: auth_session.user.github_login,
                    github_name: auth_session.user.github_name,
                    github_avatar_url: auth_session.user.github_avatar_url,
                    session_kind,
                })
            }
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "session expired").into_response()),
            Err(e) => {
                tracing::error!("failed to look up auth session: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }
}

pub fn parse_cookie<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(name) {
            let value = value.trim_start();
            if let Some(value) = value.strip_prefix('=') {
                return Some(value.trim());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cookie() {
        assert_eq!(
            parse_cookie("stiglab_session=abc123; theme=dark", "stiglab_session"),
            Some("abc123")
        );
        assert_eq!(
            parse_cookie("theme=dark; stiglab_session=xyz", "stiglab_session"),
            Some("xyz")
        );
        assert_eq!(parse_cookie("theme=dark", "stiglab_session"), None);
    }

    #[test]
    fn test_parse_cookie_empty() {
        assert_eq!(parse_cookie("", "stiglab_session"), None);
    }

    #[test]
    fn test_parse_cookie_no_value() {
        assert_eq!(
            parse_cookie("stiglab_session; theme=dark", "stiglab_session"),
            None
        );
    }

    #[test]
    fn test_parse_cookie_whitespace() {
        assert_eq!(
            parse_cookie("  stiglab_session = abc123 ; theme=dark", "stiglab_session"),
            Some("abc123")
        );
    }

    #[test]
    fn test_generate_session_token_uniqueness() {
        let t1 = generate_session_token();
        let t2 = generate_session_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 64);
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
        assert_eq!(s1.len(), 32);
    }

    #[test]
    fn session_kind_negative_github_id_is_dev() {
        assert_eq!(session_kind_for_github_id(-1), SessionKind::Dev);
        assert_eq!(session_kind_for_github_id(-9999), SessionKind::Dev);
        assert_eq!(session_kind_for_github_id(0), SessionKind::Github);
        assert_eq!(session_kind_for_github_id(123_456), SessionKind::Github);
    }

    #[test]
    fn session_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SessionKind::Github).unwrap(),
            "\"github\""
        );
        assert_eq!(serde_json::to_string(&SessionKind::Dev).unwrap(), "\"dev\"");
    }
}
