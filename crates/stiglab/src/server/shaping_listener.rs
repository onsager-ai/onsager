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
//! invariant #6). The listener uses it as the idempotency key, so
//! redelivery of the same `forge.shaping_dispatched` event collapses
//! onto a single session row instead of dispatching multiple agents.

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

struct Dispatcher {
    store: EventStore,
    app_state: AppState,
}

impl Dispatcher {
    async fn load_kind(
        &self,
        notification: &EventNotification,
    ) -> anyhow::Result<Option<FactoryEventKind>> {
        match notification.table.as_str() {
            "events" => {
                let Some(row) = self.store.get_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let envelope: FactoryEvent = serde_json::from_value(row.data)?;
                Ok(Some(envelope.event))
            }
            "events_ext" => {
                let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
                    return Ok(None);
                };
                let raw = row.data;
                if let Ok(envelope) = serde_json::from_value::<FactoryEvent>(raw.clone()) {
                    Ok(Some(envelope.event))
                } else {
                    let kind: FactoryEventKind = serde_json::from_value(raw)?;
                    Ok(Some(kind))
                }
            }
            _ => Ok(None),
        }
    }
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "forge.shaping_dispatched" {
            return Ok(());
        }

        let Some(kind) = self.load_kind(&notification).await? else {
            return Ok(());
        };

        let FactoryEventKind::ForgeShapingDispatched {
            request_id,
            artifact_id,
            request,
            ..
        } = kind
        else {
            return Ok(());
        };

        let Some(request) = request else {
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

        // Idempotency key = request_id. Forge promises stability across
        // retries (forge invariant #6); a redelivered spine notification
        // resolves to the same session row instead of dispatching twice.
        match dispatch_shaping_inner(&self.app_state, &request, &request.request_id).await {
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
