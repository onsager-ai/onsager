//! Multi-repo session-launch handoff (spec #546, Part of #544).
//!
//! A run is no longer pinned to one repo. A workspace binds an unlimited
//! set of repos (`workspace_repos`, the membership rename specced under
//! #545 / #544 and landed in PR #549), and
//! a workspace-scoped agent session is handed the *whole* bound set with
//! read-scoped per-repo credentials so the agent can clone whichever it
//! needs on demand. The single-repo / pinned case degenerates to a
//! one-element set, and a workspace with no bound repos to an empty set —
//! both reproduce today's single-`working_dir` behavior exactly.
//!
//! This module owns the pieces absorbed from #545 (its scope amendment
//! folded them here because the session launch is their first live
//! consumer — landing them in #545 would have been dangling wires):
//!
//! - [`resolve_install_for_repo`] — the per-repo install resolver.
//! - [`list_workspace_repo_installs`] — the workspace repo-set lister.
//! - [`build_workspace_repo_access`] — the eager read-scoped builder that
//!   mints one installation token per org and produces the
//!   [`RepoAccess`] handoff the `portal.session_requested` event carries.
//!
//! *Writes* to origin are out of scope: this stops at read/checkout. The
//! write-token mint lands behind the synodic gate in #548.

use std::collections::{BTreeMap, HashMap};

use onsager_github::api::app as gh_app;
use onsager_spine::protocol::RepoAccess;

use crate::auth::encrypt_credential;
use crate::core::WorkspaceRepo;
use crate::state::AppState;
use crate::workspace_db;

/// Resolve the GitHub App installation id for one specific repo bound to a
/// workspace. Returns `None` when the repo is not a member of the
/// workspace (unbound) or its membership row's install id does not parse.
///
/// Absorbed from #545's Plan: the per-repo resolver. The pure decision
/// lives in [`pick_install_for_repo`] so it is unit-testable without a DB.
pub async fn resolve_install_for_repo(
    state: &AppState,
    workspace_id: &str,
    owner: &str,
    name: &str,
) -> Option<i64> {
    let repos = workspace_db::list_projects_for_workspace(&state.pool, workspace_id)
        .await
        .unwrap_or_default();
    pick_install_for_repo(&repos, owner, name)
}

/// List every repo bound to a workspace with its resolved install id.
/// Rows whose `github_app_installation_id` does not parse are skipped —
/// they cannot mint a token, so they are not a usable member of the run's
/// repo set.
///
/// Absorbed from #545's Plan: the workspace repo-set lister.
pub async fn list_workspace_repo_installs(
    state: &AppState,
    workspace_id: &str,
) -> Vec<((String, String), i64)> {
    let repos = workspace_db::list_projects_for_workspace(&state.pool, workspace_id)
        .await
        .unwrap_or_default();
    repo_install_pairs(&repos)
}

/// Build the read-scoped repo set handed to a workspace-scoped run.
///
/// Eagerly (the #544 token posture: read is eager, write is gated) mints
/// one read-scoped installation token *per org* — grouping the workspace's
/// bound repos by install id so a multi-org workspace mints a distinct
/// token per install — encrypts each with the shared credential key, and
/// returns one [`RepoAccess`] per bound repo.
///
/// Degenerate paths reproduce today's behavior:
/// - an empty workspace (no bound repos) → empty vec;
/// - the App being unconfigured, or a mint / encrypt failure → the repos
///   are still surfaced by identity with `encrypted_read_token: None`
///   (a public repo clones unauthenticated; a private one fails loudly
///   rather than silently).
pub async fn build_workspace_repo_access(state: &AppState, workspace_id: &str) -> Vec<RepoAccess> {
    let repos = workspace_db::list_projects_for_workspace(&state.pool, workspace_id)
        .await
        .unwrap_or_default();
    if repos.is_empty() {
        return Vec::new();
    }

    // Mint one read-scoped token per install (org), encrypted at rest.
    // Missing App config / credential key / a mint failure all collapse to
    // "no token for this install" — the repos are surfaced by identity.
    let encrypted_by_install = mint_encrypted_read_tokens(state, &repos).await;
    assemble_repo_access(&repos, &encrypted_by_install)
}

/// Mint + encrypt a read-scoped token per install id present in `repos`.
/// Returns a map keyed by install id; an install absent from the map had
/// no usable token (unconfigured App, no credential key, or mint error).
async fn mint_encrypted_read_tokens(
    state: &AppState,
    repos: &[WorkspaceRepo],
) -> HashMap<i64, String> {
    let mut out = HashMap::new();

    let Some(cfg) = gh_app::AppConfig::from_env() else {
        tracing::warn!(
            "repo_access: GitHub App not configured; repos surfaced without read tokens"
        );
        return out;
    };
    let jwt = match gh_app::mint_app_jwt(&cfg) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("repo_access: mint_app_jwt failed: {e}");
            return out;
        }
    };
    let Some(key) = state.config.credential_key.as_deref() else {
        tracing::warn!("repo_access: no credential key; repos surfaced without read tokens");
        return out;
    };

    for (install_id, members) in group_repos_by_install(repos) {
        let repo_names: Vec<String> = members.iter().map(|r| r.repo_name.clone()).collect();
        match gh_app::mint_read_token_for_repos(&jwt, install_id, &repo_names).await {
            Ok(token) => match encrypt_credential(key, &token.token) {
                Ok(enc) => {
                    out.insert(install_id, enc);
                }
                Err(e) => {
                    tracing::error!(
                        "repo_access: encrypt read token for install {install_id}: {e}"
                    );
                }
            },
            Err(e) => {
                tracing::error!("repo_access: mint read token for install {install_id}: {e}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Pure helpers — DB-free and network-free so they are unit-testable.
// ---------------------------------------------------------------------------

/// Pure per-repo install pick for [`resolve_install_for_repo`].
fn pick_install_for_repo(repos: &[WorkspaceRepo], owner: &str, name: &str) -> Option<i64> {
    repos
        .iter()
        .find(|r| r.repo_owner == owner && r.repo_name == name)
        .and_then(|r| r.github_app_installation_id.parse::<i64>().ok())
}

/// Pure transform for [`list_workspace_repo_installs`]: one
/// `((owner, name), install_id)` pair per bound repo, skipping rows whose
/// install id does not parse.
fn repo_install_pairs(repos: &[WorkspaceRepo]) -> Vec<((String, String), i64)> {
    repos
        .iter()
        .filter_map(|r| {
            r.github_app_installation_id
                .parse::<i64>()
                .ok()
                .map(|id| ((r.repo_owner.clone(), r.repo_name.clone()), id))
        })
        .collect()
}

/// Group bound repos by install id (deterministic order). Rows whose
/// install id does not parse are dropped — they cannot mint a token.
fn group_repos_by_install(repos: &[WorkspaceRepo]) -> BTreeMap<i64, Vec<&WorkspaceRepo>> {
    let mut by_install: BTreeMap<i64, Vec<&WorkspaceRepo>> = BTreeMap::new();
    for r in repos {
        if let Ok(id) = r.github_app_installation_id.parse::<i64>() {
            by_install.entry(id).or_default().push(r);
        }
    }
    by_install
}

/// Assemble one [`RepoAccess`] per bound repo, attaching its install's
/// encrypted read token (if one was minted). Pure so the
/// repo→token mapping is unit-testable without minting against GitHub.
fn assemble_repo_access(
    repos: &[WorkspaceRepo],
    encrypted_by_install: &HashMap<i64, String>,
) -> Vec<RepoAccess> {
    repos
        .iter()
        .map(|r| {
            let token = r
                .github_app_installation_id
                .parse::<i64>()
                .ok()
                .and_then(|id| encrypted_by_install.get(&id).cloned());
            RepoAccess {
                owner: r.repo_owner.clone(),
                name: r.repo_name.clone(),
                default_branch: r.default_branch.clone(),
                encrypted_read_token: token,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn repo(owner: &str, name: &str, install: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            id: format!("repo_{owner}_{name}"),
            workspace_id: "w1".into(),
            github_app_installation_id: install.into(),
            repo_owner: owner.into(),
            repo_name: name.into(),
            default_branch: "main".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn pick_install_for_repo_matches_the_bound_row() {
        let repos = vec![
            repo("acme", "widgets", "11"),
            repo("acme", "gadgets", "11"),
            repo("other", "thing", "22"),
        ];
        // Mutation guard: break the `owner == && name ==` match and this
        // returns the wrong install (or None) and the assert fails.
        assert_eq!(pick_install_for_repo(&repos, "acme", "gadgets"), Some(11));
        assert_eq!(pick_install_for_repo(&repos, "other", "thing"), Some(22));
        // Unbound repo → None.
        assert_eq!(pick_install_for_repo(&repos, "acme", "nope"), None);
    }

    #[test]
    fn repo_install_pairs_lists_one_per_bound_repo() {
        let repos = vec![
            repo("acme", "widgets", "11"),
            repo("acme", "gadgets", "11"),
            repo("other", "thing", "22"),
            // Unparseable install id is skipped.
            repo("bad", "row", "not-a-number"),
        ];
        let pairs = repo_install_pairs(&repos);
        assert_eq!(pairs.len(), 3);
        assert!(pairs.contains(&(("acme".into(), "widgets".into()), 11)));
        assert!(pairs.contains(&(("other".into(), "thing".into()), 22)));
        assert!(!pairs.iter().any(|((o, _), _)| o == "bad"));
    }

    #[test]
    fn group_repos_by_install_groups_per_org() {
        let repos = vec![
            repo("acme", "widgets", "11"),
            repo("acme", "gadgets", "11"),
            repo("other", "thing", "22"),
        ];
        let grouped = group_repos_by_install(&repos);
        assert_eq!(grouped.len(), 2, "two distinct installs (orgs)");
        assert_eq!(grouped[&11].len(), 2);
        assert_eq!(grouped[&22].len(), 1);
    }

    #[test]
    fn assemble_repo_access_attaches_per_install_token() {
        let repos = vec![
            repo("acme", "widgets", "11"),
            repo("acme", "gadgets", "11"),
            repo("other", "thing", "22"),
        ];
        let mut enc = HashMap::new();
        enc.insert(11, "enc_acme".to_string());
        // Install 22 minted no token — that repo is surfaced tokenless.
        let access = assemble_repo_access(&repos, &enc);
        assert_eq!(access.len(), 3);
        let acme_widgets = access.iter().find(|a| a.name == "widgets").unwrap();
        assert_eq!(
            acme_widgets.encrypted_read_token.as_deref(),
            Some("enc_acme")
        );
        assert_eq!(acme_widgets.default_branch, "main");
        let other = access.iter().find(|a| a.owner == "other").unwrap();
        assert_eq!(other.encrypted_read_token, None);
    }
}
