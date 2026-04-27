//! Spine listener that consumes `forge.shaping_dispatched` events and
//! spawns a session via the same path the `POST /api/shaping` HTTP route
//! uses (spec #131 / ADR 0004 Lever C, phase 3).
//!
//! Replaces the legacy `forge → POST /api/shaping` HTTP call with an
//! event-driven flow. The forge producer is enriched in the same phase
//! to carry the full `ShapingRequest` payload as
//! `FactoryEventKind::ForgeShapingDispatched.request`; the listener
//! spawns a session and returns. Result correlation flows back via
//! `stiglab.shaping_result_ready` from the agent message handler when
//! the session reaches a terminal state.
//!
//! The HTTP route stays alive during phase 3 so existing forge
//! deployments with the legacy synchronous dispatcher don't break;
//! phase 5 deletes both the route and the legacy dispatcher.
//!
//! ## Idempotency
//!
//! Forge stamps a stable `request_id` on every emission (forge
//! invariant #6). The listener uses the **outer envelope's**
//! `request_id` as the idempotency key after validating it matches the
//! embedded payload — see `handle` for the divergence check.
//!
//! ## Trust model for `created_by`
//!
//! The HTTP `POST /api/shaping` route gates `created_by` behind a
//! shared secret (`X-Onsager-Internal-Dispatch`) because the endpoint
//! is publicly reachable on Railway. The listener path has no headers
//! and no shared secret — but events on the spine come from
//! processes inside the trust boundary. We require the event metadata
//! `actor == "forge"` before honoring `created_by`. Anything else
//! gets `created_by` stripped to `None` (which downgrades the dispatch
//! to a no-credentials session that fails loudly via
//! `stiglab.session_failed` rather than executing an unauthenticated
//! request the operator didn't intend).

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use crate::server::routes::shaping::{dispatch_shaping_inner, DispatchOutcome};
use crate::server::state::AppState;

/// Run the shaping listener forever. Returns only if the underlying
/// pg_notify channel closes.
///
/// `since` is the backfill cursor — pass `EventStore::max_event_id()` so
/// a fresh boot doesn't replay every historical request. Phase 6 will
/// persist a per-process cursor so a restart mid-decision doesn't drop
/// in-flight shaping dispatches.
pub async fn run(store: EventStore, app_state: AppState, since: Option<i64>) -> anyhow::Result<()> {
    let handler = Dispatcher {
        store: store.clone(),
        app_state,
    };
    Listener::new(store).with_since(since).run(handler).await
}

/// Parsed envelope: the typed event payload plus the `actor` extracted
/// from the spine row's metadata. The actor is the trust handle used
/// to gate sensitive request fields — see module-level "Trust model".
struct ParsedEvent {
    kind: FactoryEventKind,
    actor: Option<String>,
}

struct Dispatcher {
    store: EventStore,
    app_state: AppState,
}

impl Dispatcher {
    /// Load the event payload + actor for a notification. Mirrors
    /// `session_listener::Dispatcher::load_session_completed` in
    /// `forge`: malformed payloads return `Ok(None)` with a structured
    /// `warn!` instead of bubbling an error, so a single bad event
    /// doesn't spam `EventHandler error` lines.
    async fn load_event(
        &self,
        notification: &EventNotification,
    ) -> anyhow::Result<Option<ParsedEvent>> {
        match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                match serde_json::from_value::<FactoryEvent>(row.data) {
                    Ok(envelope) => Ok(Some(ParsedEvent {
                        kind: envelope.event,
                        actor: extract_actor(&row.metadata),
                    })),
                    Err(e) => {
                        tracing::warn!(
                            id = notification.id,
                            event_type = %notification.event_type,
                            "stiglab: forge.shaping_dispatched (events table) parse failed: {e}"
                        );
                        Ok(None)
                    }
                }
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let raw = row.data;
                if let Ok(envelope) = serde_json::from_value::<FactoryEvent>(raw.clone()) {
                    return Ok(Some(ParsedEvent {
                        kind: envelope.event,
                        actor: extract_actor(&row.metadata),
                    }));
                }
                match serde_json::from_value::<FactoryEventKind>(raw) {
                    Ok(kind) => Ok(Some(ParsedEvent {
                        kind,
                        actor: extract_actor(&row.metadata),
                    })),
                    Err(e) => {
                        tracing::warn!(
                            id = notification.id,
                            event_type = %notification.event_type,
                            "stiglab: forge.shaping_dispatched (events_ext) parse failed: {e}"
                        );
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }
}

/// Pull the `actor` field out of an `EventMetadata` JSON blob. Returns
/// `None` if the metadata is missing the field or isn't an object;
/// callers treat absence the same as "unknown actor" for the trust
/// gate below.
fn extract_actor(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("actor")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "forge.shaping_dispatched" {
            return Ok(());
        }

        let Some(parsed) = self.load_event(&notification).await? else {
            return Ok(());
        };

        let FactoryEventKind::ForgeShapingDispatched {
            request_id,
            artifact_id,
            request,
            ..
        } = parsed.kind
        else {
            return Ok(());
        };

        let Some(mut request) = request else {
            // Pre-phase-3 events lacked the embedded payload; without it
            // we can't construct the dispatch. Skip and log so a replay
            // of historical events doesn't hard-error.
            tracing::warn!(
                request_id = %request_id,
                artifact_id = %artifact_id,
                "stiglab: forge.shaping_dispatched has no embedded request payload, skipping"
            );
            return Ok(());
        };

        // Envelope ↔ payload divergence guard: the outer event already
        // carries `request_id`. If it disagrees with the embedded
        // payload, refuse the dispatch — a buggy producer or a partial
        // upgrade could otherwise create multiple sessions or log
        // misleading correlation.
        if request.request_id != request_id {
            tracing::error!(
                request_id = %request_id,
                embedded_request_id = %request.request_id,
                artifact_id = %artifact_id,
                "stiglab: forge.shaping_dispatched request_id mismatch between envelope \
                 and embedded payload, skipping"
            );
            return Ok(());
        }

        // Trust gate for `created_by` — see module-level "Trust model".
        // Strip it unless the spine row's metadata actor proves the
        // event came from forge. The ACL is intentionally a string
        // match: any subsystem can invent its own actor string, so the
        // check is necessary but not sufficient — the deploy
        // boundary is the actual security boundary. We log the
        // strip so an operator can see when a misbehaving producer
        // tried to elevate.
        if request.created_by.is_some() && parsed.actor.as_deref() != Some("forge") {
            tracing::warn!(
                request_id = %request_id,
                artifact_id = %artifact_id,
                actor = ?parsed.actor,
                "stiglab: stripping created_by on spine-sourced shaping_dispatched \
                 (actor != \"forge\")"
            );
            request.created_by = None;
        }

        // Idempotency key = outer envelope `request_id`. The
        // embedded payload has been verified to agree with it above.
        match dispatch_shaping_inner(&self.app_state, &request, &request_id).await {
            Ok(DispatchOutcome::Created(session)) => {
                tracing::info!(
                    request_id = %request_id,
                    artifact_id = %artifact_id,
                    session_id = %session.id,
                    "stiglab: dispatched session from forge.shaping_dispatched"
                );
            }
            Ok(DispatchOutcome::Idempotent(session)) => {
                tracing::debug!(
                    request_id = %request_id,
                    artifact_id = %artifact_id,
                    session_id = %session.id,
                    "stiglab: forge.shaping_dispatched is idempotent hit on existing session"
                );
            }
            Ok(DispatchOutcome::NoAvailableNode) => {
                // No agent connected — nothing parks the dispatch yet.
                // The artifact stays at its current stage; the next
                // emission (or a follow-up `forge` tick that re-issues
                // the dispatch) will retry once an agent comes online.
                // Phase 6 plans to persist parked dispatches; for now,
                // log loudly so operators can correlate with stalled
                // workflow artifacts.
                tracing::warn!(
                    request_id = %request_id,
                    artifact_id = %artifact_id,
                    "stiglab: forge.shaping_dispatched dropped (no available nodes)"
                );
            }
            Err(e) => {
                tracing::error!(
                    request_id = %request_id,
                    artifact_id = %artifact_id,
                    "stiglab: forge.shaping_dispatched dispatch failed: {e}"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_actor_from_typical_metadata() {
        let meta = serde_json::json!({"actor": "forge", "correlation_id": "x"});
        assert_eq!(extract_actor(&meta), Some("forge".into()));
    }

    #[test]
    fn extract_actor_returns_none_for_missing_field() {
        let meta = serde_json::json!({"correlation_id": "x"});
        assert_eq!(extract_actor(&meta), None);
    }

    #[test]
    fn extract_actor_returns_none_for_non_object() {
        // Defense in depth: a producer that wrote `null` for metadata
        // must not crash the listener.
        assert_eq!(extract_actor(&serde_json::Value::Null), None);
    }

    #[test]
    fn extract_actor_returns_none_for_non_string_actor() {
        // Defense in depth: a producer that wrote `actor: 42` shouldn't
        // be accepted as a trusted forge call.
        let meta = serde_json::json!({"actor": 42});
        assert_eq!(extract_actor(&meta), None);
    }
}
