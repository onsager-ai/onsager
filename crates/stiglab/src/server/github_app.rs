//! Stiglab-side facade over `onsager_github::api::app`.
//!
//! The real GitHub App helpers (JWT minting, install tokens, repo +
//! label listing, default branch) live in `onsager-github`. This file
//! re-exports the library items stiglab already uses by name and adds
//! a thin `get_installation` wrapper that maps `AccountKind` →
//! stiglab's `core::GitHubAccountType`.
//!
//! New call sites should depend on `onsager_github::api::app` directly;
//! the re-exports here exist so the existing stiglab routes don't all
//! need to update at once. They're slated for cleanup once the wider
//! refactor in #220 Sub-issue B moves these surfaces into portal.

pub use onsager_github::api::app::{
    get_repo_default_branch, list_installation_repos, list_repo_labels, mint_app_jwt,
    mint_installation_token, normalize_pem, AccessibleRepo, AppConfig, InstallationToken,
    RepoLabel,
};

use onsager_github::api::app as gh_app;
use onsager_github::error::GithubError;

use crate::core::GitHubAccountType;

/// Installation metadata mapped into stiglab's `core::GitHubAccountType`.
#[derive(Debug, Clone)]
pub struct InstallationInfo {
    pub account_login: String,
    pub account_type: GitHubAccountType,
}

/// Look up public metadata for an installation. Wraps
/// `onsager_github::api::app::get_installation` and maps the
/// library's `AccountKind` into stiglab's `core::GitHubAccountType`.
pub async fn get_installation(
    app_jwt: &str,
    install_id: i64,
) -> Result<InstallationInfo, GithubError> {
    let info = gh_app::get_installation(app_jwt, install_id).await?;
    let account_type = match info.account_kind {
        onsager_github::AccountKind::User => GitHubAccountType::User,
        onsager_github::AccountKind::Organization => GitHubAccountType::Organization,
    };
    Ok(InstallationInfo {
        account_login: info.account_login,
        account_type,
    })
}
