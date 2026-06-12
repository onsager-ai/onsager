//! Typed payload shapes carried on spine events between the two processes.
//!
//! Once a whole family of 0.1 exchange types (shaping, gate evaluation,
//! advisory insights); the 0.4 vocabulary sweep (ADR 0028 / spec #604)
//! deleted everything without a live reader. What remains is the
//! portal → engine session-launch handoff.

use serde::{Deserialize, Serialize};

// ===========================================================================
// Portal → engine: multi-repo session-launch handoff (spec #546)
// ===========================================================================

/// One repo handed to an agent session at launch, with read-scoped
/// access (spec #546, Part of #544).
///
/// A workspace-scoped run carries one entry per bound repo (resolved via
/// `list_workspace_repo_installs`); the repos may span multiple GitHub
/// orgs, each minted its own installation token. The single-repo / pinned
/// case degenerates to a one-element vec, and a workspace with no bound
/// repos to an empty vec — both reproduce today's single-`working_dir`
/// behavior exactly.
///
/// The token is **read-scoped** (`contents:read` + `metadata:read`) and
/// **encrypted** with the shared credential key — never the raw token on
/// the wire. The consumer (the engine's agent launch path) decrypts it
/// and builds the authenticated clone URL before surfacing it to the
/// agent. *Writes* to origin are out of scope here and gated separately
/// (#548).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoAccess {
    /// GitHub repo owner (org or user login).
    pub owner: String,
    /// GitHub repo name (short name, no owner prefix).
    pub name: String,
    /// The repo's default branch, so the agent can base work off it
    /// without an extra API round-trip.
    pub default_branch: String,
    /// Read-scoped GitHub App installation token, **encrypted** with the
    /// shared credential key. `None` when the App is unconfigured or the
    /// mint failed — the repo is still surfaced by identity (a public
    /// repo can be cloned unauthenticated; a private one will fail loudly
    /// rather than silently).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_read_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_access_roundtrip_and_token_wire_optional() {
        // Spec #546: the read token rides encrypted and is omitted from
        // the wire when absent (public-repo / unconfigured-App path).
        let with = RepoAccess {
            owner: "acme".into(),
            name: "widgets".into(),
            default_branch: "main".into(),
            encrypted_read_token: Some("enc_abc".into()),
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["encrypted_read_token"], "enc_abc");
        let back: RepoAccess = serde_json::from_value(json).unwrap();
        assert_eq!(back, with);

        let without = RepoAccess {
            encrypted_read_token: None,
            ..with
        };
        let json = serde_json::to_value(&without).unwrap();
        assert!(
            !json
                .as_object()
                .unwrap()
                .contains_key("encrypted_read_token"),
            "a None read token must be omitted from the wire"
        );
        // Legacy / tokenless payloads still parse (serde default).
        let parsed: RepoAccess = serde_json::from_value(serde_json::json!({
            "owner": "acme", "name": "widgets", "default_branch": "main"
        }))
        .expect("tokenless RepoAccess must deserialize");
        assert!(parsed.encrypted_read_token.is_none());
    }
}
