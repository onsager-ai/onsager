//! Spine listener: revoke a workspace-scoped token when its scope ends.
//!
//! Two grains, one listener (spec #531 session grain, spec #536 plan
//! grain):
//!
//! - **Per-session** (`session:<id>`, chat path) — minted by
//!   `session_token::mint_for_session`; revoked on the terminal session
//!   events (`session.completed` / `session.failed`). Those events key
//!   the `session_id` as their `events_ext` `stream_id`, so the id is
//!   read straight off the notification with no row fetch.
//! - **Per-plan** (`plan:<plan_id>`, scheduler path) — minted by
//!   `session_token::mint_for_plan`; revoked on the plan-terminal events
//!   (`plan.run_completed` / `plan.run_failed`). Those carry the run's
//!   `plan_id` in the body, so the listener fetches the row and reads it
//!   from the payload (falling back to stripping the `plan:` prefix off
//!   the stream_id).
//!
//! Each token carries a short expiry as a backstop; this listener tightens
//! the window — once the scope ends the token can no longer authenticate
//! to portal's MCP surface or the liveness hook.
//!
//! Portal is the only writer of `user_pats` (seam rule), so revocation
//! lives here and is driven by the spine, not an HTTP call. Sibling of the
//! `session_completed` PR-opener listener; same `events_ext` source.

use async_trait::async_trait;
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};
use sqlx::postgres::PgPool;

use crate::session_token;

/// Terminal session event types whose arrival revokes the session token.
const SESSION_TERMINAL_EVENTS: &[&str] = &["session.completed", "session.failed"];

/// Terminal plan-run event types whose arrival revokes the plan token
/// (#536).
const PLAN_TERMINAL_EVENTS: &[&str] = &["plan.run_completed", "plan.run_failed"];

/// Run the revoke-on-end listener forever. Returns only when the pg_notify
/// channel closes.
pub async fn run(pool: PgPool, store: EventStore) -> anyhow::Result<()> {
    Listener::new(store.clone())
        .run(Revoker { pool, store })
        .await
}

struct Revoker {
    pool: PgPool,
    store: EventStore,
}

impl Revoker {
    async fn revoke_session(&self, session_id: &str) {
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
    }

    async fn revoke_plan(&self, plan_id: &str) {
        match session_token::revoke_for_plan(&self.pool, plan_id).await {
            Ok(n) if n > 0 => {
                tracing::info!(plan_id = %plan_id, "revoked plan token on plan run end");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(plan_id = %plan_id, "failed to revoke plan token: {e}");
            }
        }
    }
}

/// Extract the run `plan_id` from a plan-terminal event. The scheduler
/// stamps `plan_id` in the body (authoritative) and keys the stream as
/// `plan:<plan_id>`. Prefer the body; fall back to stripping the
/// `plan:` prefix off the stream_id so a body-less / malformed row still
/// revokes idempotently.
fn plan_id_from_event(data: &serde_json::Value, stream_id: &str) -> Option<String> {
    if let Some(id) = data
        .get("plan_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(id.to_string());
    }
    stream_id
        .strip_prefix("plan:")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[async_trait]
impl EventHandler for Revoker {
    async fn handle(&self, event: EventNotification) -> anyhow::Result<()> {
        if event.table != "events_ext" {
            return Ok(());
        }

        // Per-session grain (#531). Session lifecycle events are keyed
        // by session_id as the stream, so the id is read straight off
        // the notification.
        if SESSION_TERMINAL_EVENTS.contains(&event.event_type.as_str()) {
            self.revoke_session(&event.stream_id).await;
            return Ok(());
        }

        // Per-plan grain (#536). Plan-terminal events carry the run
        // `plan_id` in the body — fetch the row and read it.
        if PLAN_TERMINAL_EVENTS.contains(&event.event_type.as_str()) {
            let data = match self.store.get_ext_event_by_id(event.id).await {
                Ok(Some(row)) => row.data,
                Ok(None) => serde_json::Value::Null,
                Err(e) => {
                    tracing::error!(
                        id = event.id,
                        "failed to load plan-terminal event body for token revoke: {e}"
                    );
                    serde_json::Value::Null
                }
            };
            if let Some(plan_id) = plan_id_from_event(&data, &event.stream_id) {
                self.revoke_plan(&plan_id).await;
            } else {
                tracing::warn!(
                    id = event.id,
                    stream_id = %event.stream_id,
                    "plan-terminal event with no resolvable plan_id — token not revoked"
                );
            }
            return Ok(());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_id_prefers_body_over_stream() {
        let data = json!({ "plan_id": "ws:github:42" });
        assert_eq!(
            plan_id_from_event(&data, "plan:ws:github:42").as_deref(),
            Some("ws:github:42")
        );
    }

    #[test]
    fn plan_id_falls_back_to_stream_prefix() {
        let data = json!({});
        assert_eq!(
            plan_id_from_event(&data, "plan:ws:github:42").as_deref(),
            Some("ws:github:42")
        );
    }

    #[test]
    fn plan_id_none_when_unresolvable() {
        assert!(plan_id_from_event(&json!({}), "not-a-plan-stream").is_none());
        assert!(plan_id_from_event(&serde_json::Value::Null, "plan:").is_none());
    }
}
