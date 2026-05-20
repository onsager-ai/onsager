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
//! Returns the number of events that were *actually* persisted (i.e.
//! not deduplicated). The poller uses this only as a diagnostic
//! signal — it advances the cursor on `Ok(_)` regardless, since a
//! dedupe means "the sibling path already covered this update".

use onsager_spine::{EventMetadata, EventStore, RoutedEvent, spine_namespace};
use serde_json::Value;

/// Emit a batch of routed events, stamping `workspace_id` on each and
/// applying `(adapter_id, external_ref)` dedup when both are set.
///
/// Errors are logged and skipped — one bad event must not stop the
/// rest of the batch. Returns the number of rows actually written
/// (skipped/deduplicated rows aren't counted).
pub async fn emit_routed_events(
    spine: &EventStore,
    events: Vec<RoutedEvent>,
    workspace_id: &str,
    actor: &str,
) -> usize {
    let mut written = 0usize;
    for ev in events {
        let mut data = match serde_json::to_value(&ev.kind) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("failed to serialize spine event: {e}");
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
                    Ok(Some(_id)) => {
                        written += 1;
                    }
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
                    }
                    Err(e) => {
                        tracing::warn!(
                            adapter_id,
                            external_ref,
                            "failed to emit deduped spine event: {e}"
                        );
                    }
                }
            }
            _ => {
                if let Err(e) = spine
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
                    tracing::warn!("failed to emit spine event: {e}");
                } else {
                    written += 1;
                }
            }
        }
    }
    written
}
