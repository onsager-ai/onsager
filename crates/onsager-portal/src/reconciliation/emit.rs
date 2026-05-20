//! Shared spine-emit pipeline for routed webhook / reconciler events.
//!
//! The webhook handler at `handlers/webhook.rs::route_workflow_events`
//! and the reconciliation poller at `scheduler.rs::tick_project` both
//! converge on this function: serialize each [`RoutedEvent`]'s kind,
//! stamp the workspace_id (#164), and append to `events_ext`. When the
//! event carries a dedup key (`adapter_id`, `external_ref`) we use
//! [`EventStore::append_ext_dedup`] — the partial unique index on
//! `events_ext (adapter_id, external_ref)` (spine migration 032)
//! collapses webhook/reconciler races to one row, silently.
//!
//! The return value separates three outcomes the poller's cursor
//! advance depends on:
//!   * `written` — a brand-new row landed.
//!   * `deduped` — the sibling path already wrote this resource
//!     update; treated as success because the spine is consistent.
//!   * `failed` — a DB write returned an error (caller should retry
//!     by *not* advancing the cursor).
//!
//! The reconciler's "advance only on successful emit" contract uses
//! `failed == 0` as the gate; treating dedup as success is correct
//! because the row already exists.

use onsager_spine::{EventMetadata, EventStore, RoutedEvent, spine_namespace};
use serde_json::Value;

/// Outcome of a batch emit. Counts are disjoint: every event lands
/// in exactly one of `written`, `deduped`, or `failed`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmitOutcome {
    /// New rows successfully inserted.
    pub written: usize,
    /// Inserts collapsed by the partial unique index — the sibling
    /// path (webhook ↔ reconciler) already wrote the same resource
    /// update. Treated as success for cursor-advance purposes.
    pub deduped: usize,
    /// DB writes that errored. When `> 0` the caller (poller) MUST
    /// NOT advance its reconciliation cursor — the next tick has to
    /// retry the same window.
    pub failed: usize,
}

impl EmitOutcome {
    /// All events landed (no errors). Dedup counts as success.
    pub fn all_succeeded(&self) -> bool {
        self.failed == 0
    }
}

/// Emit a batch of routed events, stamping `workspace_id` on each and
/// applying `(adapter_id, external_ref)` dedup when both are set.
///
/// Errors are logged AND counted into [`EmitOutcome::failed`] so the
/// caller can decide whether to advance its cursor. We intentionally
/// keep going on the first failure — one bad event must not stop the
/// rest of the batch from landing (the unaffected events will still
/// dedup correctly on a future retry).
pub async fn emit_routed_events(
    spine: &EventStore,
    events: Vec<RoutedEvent>,
    workspace_id: &str,
    actor: &str,
) -> EmitOutcome {
    let mut outcome = EmitOutcome::default();
    for ev in events {
        let mut data = match serde_json::to_value(&ev.kind) {
            Ok(v) => v,
            Err(e) => {
                // Encoding the in-process enum should not fail; if it
                // does we count it as a failure so the cursor doesn't
                // advance over an event we never tried to write.
                tracing::error!("failed to serialize spine event: {e}");
                outcome.failed += 1;
                continue;
            }
        };
        // Stamp workspace_id (#164) so downstream consumers — including
        // the workspace-scoped `/api/spine/events` listing — can filter
        // by workspace without re-resolving the install. The
        // `TriggerFired` payload already includes its workflow's
        // workspace; we don't overwrite it (a missing entry is the
        // common case for `gate.*` events from check / PR webhooks).
        if let Some(obj) = data.as_object_mut() {
            obj.entry("workspace_id".to_string())
                .or_insert(Value::String(workspace_id.to_string()));
        }
        let namespace = spine_namespace(&ev.kind);
        let stream_id = ev.kind.stream_id();
        let event_type = ev.kind.event_type();
        let metadata = EventMetadata {
            correlation_id: None,
            causation_id: None,
            actor: actor.to_string(),
        };

        match (ev.adapter_id.as_deref(), ev.external_ref.as_deref()) {
            (Some(adapter_id), Some(external_ref)) => {
                match spine
                    .append_ext_dedup(
                        workspace_id,
                        &stream_id,
                        namespace,
                        event_type,
                        data,
                        &metadata,
                        adapter_id,
                        external_ref,
                    )
                    .await
                {
                    Ok(Some(_id)) => outcome.written += 1,
                    Ok(None) => {
                        // Sibling path (webhook ↔ reconciler) already
                        // wrote this resource update — silent no-op
                        // per the dedup contract.
                        tracing::debug!(
                            adapter_id,
                            external_ref,
                            event_type,
                            "spine event deduplicated against existing (adapter_id, external_ref)"
                        );
                        outcome.deduped += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            adapter_id,
                            external_ref,
                            "failed to emit deduped spine event: {e}"
                        );
                        outcome.failed += 1;
                    }
                }
            }
            _ => match spine
                .append_ext(
                    workspace_id,
                    &stream_id,
                    namespace,
                    event_type,
                    data,
                    &metadata,
                    None,
                )
                .await
            {
                Ok(_) => outcome.written += 1,
                Err(e) => {
                    tracing::warn!("failed to emit spine event: {e}");
                    outcome.failed += 1;
                }
            },
        }
    }
    outcome
}
