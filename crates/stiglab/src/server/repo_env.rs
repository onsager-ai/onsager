//! Surface the run's multi-repo set to the agent (spec #546, Part of #544).
//!
//! Portal resolves the workspace's bound repo set with read-scoped
//! per-repo credentials and ships it on the `portal.session_requested`
//! event as `Vec<RepoAccess>` (tokens encrypted with the shared
//! credential key). Stiglab is the consumer: it decrypts each token and
//! exposes the set to the agent through the `ONSAGER_REPOS` env var — a
//! JSON array the agent reads to clone whichever repos it needs beneath
//! its `working_dir`.
//!
//! The single-repo / pinned case is a one-element array; a workspace with
//! no bound repos surfaces nothing (`None`), leaving the existing
//! single-`working_dir` behavior untouched. *Writes* to origin are out of
//! scope — these are read/checkout credentials, gated separately (#548).
//!
//! Spec #555 extended the same env var to the scheduler/`AgentExecutor`
//! launch path. The agent-facing JSON shape is the contract both paths
//! must produce identically, so it moved to `onsager-agent-spawn`
//! ([`onsager_agent_spawn::repos_env_json`]); this module keeps only
//! stiglab's spine→agent decrypt boundary.

use onsager_spine::protocol::RepoAccess;

use crate::server::auth::decrypt_credential;

// The agent-facing wire shape (`ClonableRepo` → `ONSAGER_REPOS` JSON) is
// the contract shared with the scheduler launch path, so it lives in
// `onsager-agent-spawn` (spec #555). Stiglab keeps only the spine→agent
// decrypt boundary below — mapping each encrypted `RepoAccess` onto a
// plaintext `ClonableRepo` across its own `decrypt_credential`.
pub use onsager_agent_spawn::{ClonableRepo, REPOS_ENV};

/// Decrypt the wire repo set and build the `ONSAGER_REPOS` env value.
///
/// Returns `None` when the set is empty (single/zero-repo path injects
/// nothing). A token that fails to decrypt (or no credential key) degrades
/// to an unauthenticated clone URL for that repo rather than dropping it.
pub fn repos_env_from_access(repos: &[RepoAccess], key: Option<&str>) -> Option<String> {
    if repos.is_empty() {
        return None;
    }
    let clonable: Vec<ClonableRepo> = repos
        .iter()
        .map(|r| {
            let token = match (key, r.encrypted_read_token.as_deref()) {
                (Some(k), Some(enc)) => match decrypt_credential(k, enc) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        // Observability: a decrypt failure (mismatched /
                        // rotated credential key) silently degrades a
                        // private-repo clone to an unauthenticated URL,
                        // which then fails far from here. Surface it — the
                        // error is about the cipher, never the plaintext
                        // token, so it is safe to log.
                        tracing::warn!(
                            owner = %r.owner,
                            repo = %r.name,
                            "repo_env: failed to decrypt read token; surfacing \
                             unauthenticated clone URL: {e}"
                        );
                        None
                    }
                },
                _ => None,
            };
            ClonableRepo {
                owner: r.owner.clone(),
                name: r.name.clone(),
                default_branch: r.default_branch.clone(),
                token,
            }
        })
        .collect();
    onsager_agent_spawn::repos_env_json(&clonable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::auth::encrypt_credential;

    // A 32-byte key (64 hex chars) for the AES-256-GCM credential cipher.
    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn access(owner: &str, name: &str, key: &str, raw_token: Option<&str>) -> RepoAccess {
        RepoAccess {
            owner: owner.into(),
            name: name.into(),
            default_branch: "main".into(),
            encrypted_read_token: raw_token.map(|t| encrypt_credential(key, t).unwrap()),
        }
    }

    #[test]
    fn two_repos_surface_both_with_authenticated_clone_urls() {
        // Spec #546 integration check (the surfacing seam): a workspace-
        // scoped run with 2 bound repos surfaces one authenticated entry
        // per repo. Mutation guard: drop one repo from the input and the
        // length assertion below fails.
        let repos = vec![
            access("acme", "widgets", KEY, Some("tok_a")),
            access("other", "thing", KEY, Some("tok_b")),
        ];
        let json = repos_env_from_access(&repos, Some(KEY)).expect("two repos surface a value");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2, "one entry per bound repo");
        assert_eq!(
            arr[0]["clone_url"],
            "https://x-access-token:tok_a@github.com/acme/widgets.git"
        );
        assert_eq!(
            arr[1]["clone_url"],
            "https://x-access-token:tok_b@github.com/other/thing.git"
        );
        assert_eq!(arr[0]["default_branch"], "main");
    }

    #[test]
    fn single_repo_surfaces_exactly_one_entry() {
        // Regression: the single-repo path is unchanged — exactly one
        // entry, same authenticated shape.
        let repos = vec![access("acme", "widgets", KEY, Some("tok_a"))];
        let json = repos_env_from_access(&repos, Some(KEY)).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 1);
    }

    #[test]
    fn empty_repo_set_surfaces_nothing() {
        // Regression: zero bound repos → no ONSAGER_REPOS env injected, so
        // the existing single-`working_dir` behavior is untouched.
        assert_eq!(repos_env_from_access(&[], Some(KEY)), None);
    }

    #[test]
    fn missing_token_degrades_to_unauthenticated_url() {
        // No credential key (or an undecryptable token) → the repo is
        // still surfaced, just with a plain HTTPS clone URL.
        let repos = vec![access("acme", "public", KEY, None)];
        let json = repos_env_from_access(&repos, Some(KEY)).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr[0]["clone_url"], "https://github.com/acme/public.git");
    }
}
