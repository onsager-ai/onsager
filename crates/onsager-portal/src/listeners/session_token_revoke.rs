//! Spine listener: revoke a session's workspace-scoped token when the
//! session ends (spec #531).
//!
//! Portal mints a per-session PAT at session creation
//! (`session_token::mint_for_session`); the token carries a short expiry as
//! a backstop. This listener tightens the window — on any terminal session
//! event it soft-revokes the token immediately, so a finished session can
//! no longer authenticate to portal's MCP surface or the liveness hook.
//!
//! Portal is the only writer of `user_pats` (seam rule), so revocation
//! lives here and is driven by the spine, not an HTTP call. Sibling of the
//! `session_completed` PR-opener listener; same `events_ext` source.
//!
//! The terminal events all use the `session_id` as their `events_ext`
//! `stream_id` (see `FactoryEventKind::stream_id`), so the id is read
//! straight off the notification with no row fetch.

use async_trait::async_trait;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use sqlx::postgres::PgPool;

use crate::session_token;

/// Terminal session event types whose arrival revokes the session token.
const TERMINAL_EVENTS: &[&str] = &[
    "stiglab.session_completed",
    "stiglab.session_failed",
    "stiglab.session_aborted",
];

/// Run the revoke-on-end listener forever. Returns only when the pg_notify
/// channel closes.
pub async fn run(pool: PgPool, store: EventStore) -> anyhow::Result<()> {
    Listener::new(store).run(Revoker { pool }).await
}

struct Revoker {
    pool: PgPool,
}

#[async_trait]
impl EventHandler for Revoker {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        // Session lifecycle events are stiglab-namespaced `events_ext` rows
        // (see stiglab's `SpineEmitter::emit`), keyed by session_id as the
        // stream.
        if event.table != "events_ext" || !TERMINAL_EVENTS.contains(&event.event_type.as_str()) {
            return Ok(());
        }
        let session_id = &event.stream_id;
        match session_token::revoke_for_session(&self.pool, session_id).await {
            Ok(n) if n > 0 => {
                tracing::info!(session_id = %session_id, "revoked session token on session end");
            }
            // No live token (unscoped session, already revoked, or never
            // minted) — nothing to do.
            Ok(_) => {}
            Err(e) => {
                tracing::error!(session_id = %session_id, "failed to revoke session token: {e}");
            }
        }
        Ok(())
    }
}
