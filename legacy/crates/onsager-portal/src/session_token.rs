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
//!   machinery); the raw token is handed straight to the in-process
//!   session runner (#583), which injects it as `ONSAGER_SESSION_TOKEN`
//!   — it never rides a spine event, so nothing usable is at rest.
//! - **Revoke** when the session ends ([`session_pat_name`] + the
//!   session-lifecycle listener): the token is soft-revoked on
//!   `session.completed` / `session.failed` so the window closes
//!   immediately rather than waiting for expiry.
//!
//! Per spec #536 the same machinery provisions a **plan-scoped** token
//! for the substrate-scheduler dispatch path: a single token named
//! `plan:<plan_id>`, minted by portal's `run_spec_plan` tool, reused
//! across every agent node in the plan, and revoked when the plan
//! reaches a terminal state (`plan.run_completed` / `plan.run_failed`).
//! The chat path (`portal.session_requested`) keeps its per-session
//! grain; the scheduler path uses the per-plan grain.
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

/// The `user_pats.name` for a plan-scoped token (spec #536). Plan ids are
/// unique per run (`derive_plan_id(workspace, spec_plan)`), so this is a
/// unique handle the revocation path matches on. Distinct prefix from
/// `session:` so a single PAT row keys exactly one grain.
pub fn plan_pat_name(plan_id: &str) -> String {
    format!("plan:{plan_id}")
}

/// Mint a workspace-pinned PAT under `name` and return the **raw**
/// token. Only the hash is persisted to `user_pats`. Shared body of the
/// per-session ([`mint_for_session`]) and per-plan ([`mint_for_plan`])
/// minters; the plan minter encrypts the result before it crosses the
/// spine, the session minter hands it straight to the in-process runner.
async fn mint_named(
    pool: &PgPool,
    user_id: &str,
    workspace_id: &str,
    name: &str,
) -> anyhow::Result<String> {
    let pat = generate_pat_token();
    let expires_at = Utc::now() + SESSION_TOKEN_TTL;
    pat_db::insert_user_pat(
        pool,
        &Uuid::new_v4().to_string(),
        user_id,
        workspace_id,
        name,
        &pat.prefix,
        &pat.hash,
        Some(expires_at),
    )
    .await?;
    Ok(pat.token)
}

/// Mint a workspace-pinned PAT for `session_id` and return the **raw**
/// token. Only the hash is persisted to `user_pats`. Per #583 the
/// session runner lives in-process, so the token never crosses the
/// spine and needs no encrypt→decrypt round-trip (the retired
/// `portal.session_requested` wire carried it encrypted).
pub async fn mint_for_session(
    pool: &PgPool,
    user_id: &str,
    workspace_id: &str,
    session_id: &str,
) -> anyhow::Result<String> {
    mint_named(pool, user_id, workspace_id, &session_pat_name(session_id)).await
}

/// Mint a workspace-pinned PAT for `plan_id` (spec #536) and return the
/// raw token **encrypted** with `key_hex` (ready to embed in the
/// `plan.run_requested` event). Reused across every agent node in the
/// plan; revoked on the plan-terminal event. Only the hash is persisted.
pub async fn mint_for_plan(
    pool: &PgPool,
    key_hex: &str,
    user_id: &str,
    workspace_id: &str,
    plan_id: &str,
) -> anyhow::Result<String> {
    let raw = mint_named(pool, user_id, workspace_id, &plan_pat_name(plan_id)).await?;
    encrypt_credential(key_hex, &raw)
}

/// Revoke the session-scoped PAT for `session_id` (idempotent). Returns the
/// number of rows revoked (0 if already revoked / never minted).
pub async fn revoke_for_session(pool: &PgPool, session_id: &str) -> anyhow::Result<u64> {
    pat_db::revoke_pats_by_name(pool, &session_pat_name(session_id)).await
}

/// Revoke the plan-scoped PAT for `plan_id` (spec #536, idempotent).
/// Returns the number of rows revoked (0 if already revoked / never
/// minted). Driven by the plan-terminal spine listener.
pub async fn revoke_for_plan(pool: &PgPool, plan_id: &str) -> anyhow::Result<u64> {
    pat_db::revoke_pats_by_name(pool, &plan_pat_name(plan_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_pat_name_is_prefixed() {
        assert_eq!(session_pat_name("abc-123"), "session:abc-123");
    }

    #[test]
    fn plan_pat_name_is_prefixed() {
        assert_eq!(plan_pat_name("ws:github:42"), "plan:ws:github:42");
    }
}
