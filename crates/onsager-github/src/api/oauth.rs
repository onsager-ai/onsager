//! GitHub OAuth helpers.
//!
//! Three steps: build the authorize URL, exchange the returned code
//! for an access token, fetch the GitHub user behind the token. The
//! library owns the GitHub-specific exchange; the surrounding
//! session-cookie / principal logic stays at the call site.

use serde::Deserialize;

use crate::api::http::client;
use crate::error::GithubError;

#[derive(Debug, Deserialize)]
pub struct GithubTokenResponse {
    pub access_token: String,
}

/// GitHub user identity returned by `GET /user`. Field set is what
/// stiglab's `auth.rs` consumed today (id + login + name + avatar) —
/// nothing extra.
#[derive(Debug, Deserialize)]
pub struct GithubOAuthUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Build the authorize URL for the OAuth web flow. `scope` is hardcoded
/// to `read:user` — every Onsager OAuth surface today only needs the
/// public-profile scope.
pub fn github_authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let mut url = reqwest::Url::parse("https://github.com/login/oauth/authorize")
        .expect("hardcoded GitHub OAuth authorize URL must be valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("scope", "read:user");
    url.into()
}

/// Exchange an OAuth `code` for an access token.
pub async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<GithubTokenResponse, GithubError> {
    let resp = client()
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": code,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(GithubError::from_response(resp).await);
    }

    resp.json()
        .await
        .map_err(|e| GithubError::Decode(e.to_string()))
}

/// Fetch the GitHub user behind an OAuth access token.
pub async fn get_oauth_user(access_token: &str) -> Result<GithubOAuthUser, GithubError> {
    let resp = client()
        .get("https://api.github.com/user")
        .bearer_auth(access_token)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(GithubError::from_response(resp).await);
    }

    resp.json()
        .await
        .map_err(|e| GithubError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_scope_and_state() {
        let url = github_authorize_url("client123", "https://example.com/callback", "state456");
        assert!(url.contains("client_id=client123"));
        assert!(url.contains("state=state456"));
        assert!(url.contains("scope=read%3Auser"));
        assert!(url.starts_with("https://github.com/login/oauth/authorize"));
    }
}
