//! Surface the run's multi-repo set to the scheduler-launched agent
//! (spec #555, parity with #546).
//!
//! Multi-repo read (#546 / PR #552) was first wired on stiglab's
//! `portal.session_requested` path. The scheduler/`AgentExecutor` launch
//! path — where spec-plan and automated workflow runs execute — had no
//! repo-set wiring at all. This module closes that gap: portal attaches
//! the workspace's bound repo set (`Vec<RepoAccess>`, tokens encrypted
//! with the shared credential key) onto the `plan.run_requested` event,
//! exactly as it already attaches the plan-scoped session token (#536);
//! the scheduler decrypts each token and exposes the set to the agent
//! through `ONSAGER_REPOS`.
//!
//! This is the scheduler's spine→agent decrypt boundary — the direct
//! counterpart of stiglab's [`crate`](../../stiglab/src/server/repo_env.rs).
//! The agent-facing JSON shape lives in
//! [`onsager_agent_spawn::repos_env_json`] so the two launch paths cannot
//! drift; the decrypt uses this crate's own [`crate::scheduler::crypto`] replica,
//! the same split the session token already uses (the seam rule forbids
//! depending on portal/stiglab). *Writes* to origin remain out of scope —
//! these are read/checkout credentials, gated separately (#548).

use onsager_agent_spawn::ClonableRepo;
use onsager_spine::protocol::RepoAccess;

use crate::scheduler::crypto::decrypt_credential;

/// Decrypt the wire repo set and build the `ONSAGER_REPOS` env value.
///
/// Returns `None` when the set is empty (single/zero-repo scheduler runs
/// inject nothing, so they are unchanged). A token that fails to decrypt
/// (or an absent credential key) degrades to an unauthenticated clone URL
/// for that repo rather than dropping it — matching stiglab's behavior.
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
                        // A decrypt failure (mismatched / rotated key)
                        // silently degrades a private-repo clone to an
                        // unauthenticated URL, which then fails far from
                        // here. Surface it — the error is about the
                        // cipher, never the plaintext, so it is safe to
                        // log.
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
    use ring::aead;
    use ring::rand::{SecureRandom, SystemRandom};

    // A 32-byte key (64 hex chars) for the AES-256-GCM credential cipher.
    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    /// Encrypt mirroring portal's `encrypt_credential` so the decrypt
    /// round-trip is provable without depending on portal. Production
    /// ciphertext always comes from portal (which mints the read token);
    /// this only exercises that the scheduler reads that exact wire
    /// format.
    fn encrypt(key_hex: &str, plaintext: &str) -> String {
        let key_bytes = hex::decode(key_hex).unwrap();
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes).unwrap();
        let sealing = aead::LessSafeKey::new(unbound);
        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes).unwrap();
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();
        sealing
            .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
            .unwrap();
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        hex::encode(result)
    }

    fn access(owner: &str, name: &str, raw_token: Option<&str>) -> RepoAccess {
        RepoAccess {
            owner: owner.into(),
            name: name.into(),
            default_branch: "main".into(),
            encrypted_read_token: raw_token.map(|t| encrypt(KEY, t)),
        }
    }

    #[test]
    fn two_repos_surface_both_with_authenticated_clone_urls() {
        // Spec #555 integration check (the scheduler-path surfacing seam):
        // a scheduler-launched run for a workspace with 2 bound repos
        // surfaces one authenticated entry per repo, decrypted off the
        // wire token. Mutation guard: drop one repo from the input and the
        // length assertion below fails.
        let repos = vec![
            access("acme", "widgets", Some("tok_a")),
            access("other", "thing", Some("tok_b")),
        ];
        let json = repos_env_from_access(&repos, Some(KEY)).expect("two repos surface a value");
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = arr.as_array().unwrap();
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
    fn single_repo_scheduler_run_is_unchanged() {
        // Regression: the single-repo path surfaces exactly one entry,
        // same authenticated shape.
        let repos = vec![access("acme", "widgets", Some("tok_a"))];
        let json = repos_env_from_access(&repos, Some(KEY)).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 1);
    }

    #[test]
    fn empty_repo_set_surfaces_nothing() {
        // Regression: zero bound repos → no ONSAGER_REPOS injected, so a
        // single-`working_dir` scheduler run is untouched.
        assert_eq!(repos_env_from_access(&[], Some(KEY)), None);
    }

    #[test]
    fn missing_token_degrades_to_unauthenticated_url() {
        // No credential key (or an undecryptable token) → the repo is
        // still surfaced, just with a plain HTTPS clone URL.
        let repos = vec![access("acme", "public", None)];
        let json = repos_env_from_access(&repos, Some(KEY)).unwrap();
        let arr: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(arr[0]["clone_url"], "https://github.com/acme/public.git");
    }
}
