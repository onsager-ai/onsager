//! Credential modes for GitHub access.
//!
//! Two shapes today: an App installation (server-to-server, scoped to a
//! workspace) and a personal access token. Both implement
//! [`GithubAuth`] so call sites stay uniform.
//!
//! Resolution from raw config (env or DB row) lives at the call site —
//! the library only owns the *types* and the act of producing an
//! `Authorization` header. The intent (per #220 Sub-issue B) is to push
//! credential persistence into the portal subsystem; the type layer here
//! survives that move unchanged.

use serde::{Deserialize, Serialize};

use crate::error::GithubError;

/// Account flavor returned by the App `installations/:id` endpoint.
/// Mirrors GitHub's `User` / `Organization` distinction.
///
/// Subsystems that maintain their own enum (e.g. stiglab's
/// `core::GitHubAccountType`) map between their type and this one at
/// the boundary — keeps the library free of subsystem domain types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    User,
    Organization,
}

impl AccountKind {
    /// Map GitHub's mixed-case `"User"` / `"Organization"` (anything
    /// else collapses to `Organization`, matching the existing
    /// stiglab behavior we're preserving).
    pub fn from_github_str(s: &str) -> Self {
        match s {
            "User" => AccountKind::User,
            _ => AccountKind::Organization,
        }
    }
}

/// How to authenticate against GitHub.
#[derive(Debug, Clone)]
pub enum Credential {
    /// GitHub App installation token. Short-lived; minted from an App
    /// JWT against a specific installation.
    Installation { token: String },
    /// Personal access token (classic or fine-grained). Used for
    /// self-hosted deployments without a registered App.
    Pat { token: String },
}

impl Credential {
    /// The string suitable for the `Authorization: Bearer …` header.
    pub fn bearer(&self) -> &str {
        match self {
            Credential::Installation { token } | Credential::Pat { token } => token,
        }
    }
}

/// Anything that can produce a `Credential` for the current call.
///
/// Most call sites today resolve a credential synchronously from a
/// config struct, but installation tokens are minted via an async HTTP
/// call against an App JWT — hence `async_trait`.
#[async_trait::async_trait]
pub trait GithubAuth: Send + Sync {
    async fn credential(&self) -> Result<Credential, GithubError>;
}

#[async_trait::async_trait]
impl GithubAuth for Credential {
    async fn credential(&self) -> Result<Credential, GithubError> {
        Ok(self.clone())
    }
}
