//! MCP tool `request_repo_write` — mint an on-demand, per-repo write
//! token (spec #548, Part of #544; gate protocol retired by ADR 0027 /
//! spec #582).
//!
//! In a workspace-scoped run the agent is handed *read* access to the
//! workspace's bound repo set (`ONSAGER_REPOS`, #546). When the agent
//! decides it needs to push + open a PR to a bound-but-unpinned repo, it
//! calls this tool. Portal:
//!
//! 1. Verifies the repo is actually bound to the workspace — binding is
//!    the consent boundary for writes.
//! 2. Mints a `contents:write` + `pull_requests:write` token scoped to
//!    that one repo and returns an authenticated remote.
//!
//! The pinned repo (named in the trigger) is pre-approved — the pin *is*
//! the consent — and uses the unchanged push/PR path, so it never reaches
//! this tool.
//!
//! The gate round-trip that used to sit between candidacy and mint
//! retired with the governance subsystem (ADR 0027 / spec #582). When
//! verify gating becomes real it lands as an in-process check here,
//! not a subsystem behind events (re-scoped MIG-01 #363).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::AuthUser;
use crate::repo_access::{self, WriteTokenError};
use crate::state::AppState;

use super::super::{ToolError, ToolResult};
use super::require_workspace_access;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RequestRepoWriteArgs {
    /// Workspace that owns the run. The caller's MCP token scope is
    /// enforced against this value before anything is written.
    pub workspace_id: String,
    /// The session/run requesting write access — attributes the mint to
    /// the run in logs.
    pub session_id: String,
    /// Target repo owner (org or user login).
    pub owner: String,
    /// Target repo short name (no owner prefix).
    pub name: String,
    /// Artifact the run is shaping, when known. Optional; retained for
    /// call-site compatibility and log attribution.
    #[serde(default)]
    pub artifact_id: Option<String>,
}

/// Authenticated HTTPS remote the agent uses to `git push`. The token
/// rides in the `x-access-token` userinfo per GitHub's installation-token
/// convention, percent-encoded so a reserved character can't truncate or
/// corrupt the URL — same defensive shape the read side uses for clone
/// URLs. GitHub installation tokens are URL-safe today; encoding keeps
/// this correct if that ever changes.
fn authenticated_remote(owner: &str, name: &str, token: &str) -> String {
    format!(
        "https://x-access-token:{}@github.com/{owner}/{name}.git",
        onsager_agent_spawn::urlencode(token)
    )
}

pub async fn request_repo_write(state: &AppState, auth_user: &AuthUser, args: Value) -> ToolResult {
    let args: RequestRepoWriteArgs = serde_json::from_value(args)
        .map_err(|e| ToolError::InvalidParams(format!("invalid request_repo_write args: {e}")))?;
    require_workspace_access(&state.pool, auth_user, &args.workspace_id).await?;

    let target_repo = format!("{}/{}", args.owner, args.name);

    // Candidacy check: an unbound repo can never be written to. Binding
    // a repo to the workspace is the write-consent boundary.
    if repo_access::resolve_install_for_repo(state, &args.workspace_id, &args.owner, &args.name)
        .await
        .is_none()
    {
        return Ok(serde_json::json!({
            "granted": false,
            "status": "repo_not_bound",
            "reason": format!(
                "{target_repo} is not bound to this workspace; binding grants candidacy for writes"
            ),
        }));
    }

    tracing::info!(
        workspace_id = %args.workspace_id,
        session_id = %args.session_id,
        actor = %auth_user.user_id,
        %target_repo,
        "request_repo_write: minting write token for bound repo"
    );

    match repo_access::mint_repo_write_token(state, &args.workspace_id, &args.owner, &args.name)
        .await
    {
        // The token is returned only once, embedded in `remote`
        // (sufficient for `git push`). Returning it as a separate
        // field too would duplicate a secret and widen the chance
        // a client logs or persists it.
        Ok(token) => Ok(serde_json::json!({
            "granted": true,
            "status": "granted",
            "remote": authenticated_remote(&args.owner, &args.name, &token.token),
            "expires_at": token.expires_at,
        })),
        Err(WriteTokenError::RepoNotBound) => Ok(serde_json::json!({
            "granted": false,
            "status": "repo_not_bound",
            "reason": format!("{target_repo} is not bound to this workspace"),
        })),
        Err(e) => {
            tracing::error!("request_repo_write: write-token mint failed: {e}");
            Err(ToolError::Internal(format!("write-token mint failed: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_remote_embeds_token() {
        let remote = authenticated_remote("acme", "widgets", "ghs_tok");
        assert_eq!(
            remote,
            "https://x-access-token:ghs_tok@github.com/acme/widgets.git"
        );
    }

    #[test]
    fn authenticated_remote_percent_encodes_reserved_chars() {
        let remote = authenticated_remote("acme", "widgets", "to@k/en");
        assert_eq!(
            remote,
            "https://x-access-token:to%40k%2Fen@github.com/acme/widgets.git"
        );
    }
}
