//! Per-session workspace-scoped tokens (spec #531).
//!
//! An agent session runs in an untrusted execution environment that holds
//! no DB credentials. To let it reach portal's MCP surface (the
//! `emit_artifact` / `declare_no_output` / `read_emit_status` tools, spec
//! #520 §4a) and the liveness Stop hook (#525), the session is provisioned
//! a narrow, **workspace-pinned PAT** minted here.
//!
//! Lifecycle (locked design, spec #531):
//!
//! - **Mint** at session creation ([`mint_for_session`]): a PAT pinned to
//!   the session's workspace, named `session:<session_id>`, with a short
//!   `expires_at`. Only the hash is persisted (reusing the existing PAT
//!   machinery); the raw token is **encrypted** with the shared credential
//!   key and travels inside the `portal.session_requested` event so no
//!   usable secret is persisted in `events_ext` at rest. Stiglab decrypts
//!   it at dispatch and injects it as `ONSAGER_SESSION_TOKEN`.
//! - **Revoke** when the session ends ([`session_pat_name`] + the
//!   session-lifecycle listener): the token is soft-revoked on
//!   `stiglab.session_completed` / `_failed` / `_aborted` so the window
//!   closes immediately rather than waiting for expiry.
//!
//! Portal is the only writer of `user_pats` (seam rule), so both mint and
//! revoke live on the portal side.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{encrypt_credential, generate_pat_token};
use crate::pat_db;

/// How long a session token is valid before it expires on its own. The
/// session-lifecycle listener revokes it earlier on a terminal event; this
/// is the backstop for sessions whose terminal event is missed.
pub const SESSION_TOKEN_TTL: Duration = Duration::hours(6);

/// The `user_pats.name` for a session-scoped token. Session ids are unique,
/// so this is effectively a unique handle the revocation path matches on.
pub fn session_pat_name(session_id: &str) -> String {
    format!("session:{session_id}")
}

/// Mint a workspace-pinned PAT for `session_id` and return the raw token
/// **encrypted** with `key_hex` (ready to embed in the dispatch event).
/// Only the hash is persisted to `user_pats`.
pub async fn mint_for_session(
    pool: &PgPool,
    key_hex: &str,
    user_id: &str,
    workspace_id: &str,
    session_id: &str,
) -> anyhow::Result<String> {
    let pat = generate_pat_token();
    let expires_at = Utc::now() + SESSION_TOKEN_TTL;
    pat_db::insert_user_pat(
        pool,
        &Uuid::new_v4().to_string(),
        user_id,
        workspace_id,
        &session_pat_name(session_id),
        &pat.prefix,
        &pat.hash,
        Some(expires_at),
    )
    .await?;
    let encrypted = encrypt_credential(key_hex, &pat.token)?;
    Ok(encrypted)
}

/// Revoke the session-scoped PAT for `session_id` (idempotent). Returns the
/// number of rows revoked (0 if already revoked / never minted).
pub async fn revoke_for_session(pool: &PgPool, session_id: &str) -> anyhow::Result<u64> {
    pat_db::revoke_pats_by_name(pool, &session_pat_name(session_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_pat_name_is_prefixed() {
        assert_eq!(session_pat_name("abc-123"), "session:abc-123");
    }
}
