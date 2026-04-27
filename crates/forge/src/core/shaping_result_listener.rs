//! Listener that records `stiglab.shaping_result_ready` events into
//! [`PendingShapings`] (spec #131 / ADR 0004 Lever C, phase 3).
//!
//! Symmetric with [`crate::core::gate_verdict_listener`]: filters on
//! `event_type`, parses the typed variant, and pushes the embedded
//! `ShapingResult` into the shared map under its `request_id` key.
//!
//! The lifecycle event `stiglab.session_completed` keeps its existing
//! `SessionLinker` consumer (vertical-lineage write) — that listener
//! handles cluster-state, this one handles artifact-state. Phase 4 wires
//! the pipeline tick's resume path to `take()` from this map.

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use super::pending::PendingShapings;

/// Run the shaping-result listener forever. Returns only if pg_notify
/// closes. `since` is the backfill cursor (see
/// [`crate::core::gate_verdict_listener::run`] for the same caveat).
pub async fn run(
    store: EventStore,
    pending: PendingShapings,
    since: Option<i64>,
) -> anyhow::Result<()> {
    let handler = Dispatcher {
        store: store.clone(),
        pending,
    };
    Listener::new(store).with_since(since).run(handler).await
}

struct Dispatcher {
    store: EventStore,
    pending: PendingShapings,
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

/// Pure classifier: extract the `(request_id, ShapingResult)` pair from a
/// [`FactoryEventKind`], or `None` if the kind is not a shaping result.
///
/// Key is the `request_id` Forge stamped on the originating
/// `forge.shaping_dispatched`. The duplicate top-level `artifact_id` on
/// the event is kept for stream routing; we don't need it here.
pub fn classify_shaping_result(
    kind: FactoryEventKind,
) -> Option<(String, onsager_spine::protocol::ShapingResult)> {
    match kind {
        FactoryEventKind::StiglabShapingResultReady { result, .. } => {
            Some((result.request_id.clone(), result))
        }
        _ => None,
    }
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "stiglab.shaping_result_ready" {
            return Ok(());
        }

        let Some(kind) = self.load_kind(&notification).await? else {
            return Ok(());
        };

        if let Some((request_id, result)) = classify_shaping_result(kind) {
            tracing::info!(
                request_id = %request_id,
                "forge: parking stiglab.shaping_result_ready for pipeline resume"
            );
            self.pending.insert(&request_id, result);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::{ArtifactId, ContentRef};
    use onsager_spine::factory_event::ShapingOutcome;
    use onsager_spine::protocol::ShapingResult;

    fn shaping_completed(req: &str) -> ShapingResult {
        ShapingResult {
            request_id: req.into(),
            outcome: ShapingOutcome::Completed,
            content_ref: Some(ContentRef {
                uri: "git://repo@abc".into(),
                checksum: None,
            }),
            change_summary: "ok".into(),
            quality_signals: vec![],
            session_id: "sess".into(),
            duration_ms: 10,
            error: None,
        }
    }

    #[test]
    fn classify_extracts_request_id_and_full_result() {
        let kind = FactoryEventKind::StiglabShapingResultReady {
            artifact_id: ArtifactId::new("art_x"),
            result: shaping_completed("req_42"),
        };
        let (request_id, result) = classify_shaping_result(kind).expect("matches variant");
        assert_eq!(request_id, "req_42");
        assert_eq!(result.outcome, ShapingOutcome::Completed);
        assert_eq!(result.content_ref.unwrap().uri, "git://repo@abc");
    }

    #[test]
    fn classify_returns_none_for_session_completed() {
        // Lifecycle event — different consumer (SessionLinker writes
        // vertical_lineage). Must not be parked as a shaping result.
        let kind = FactoryEventKind::StiglabSessionCompleted {
            session_id: "s".into(),
            request_id: "r".into(),
            duration_ms: 1,
            artifact_id: Some("art_x".into()),
            token_usage: None,
            branch: None,
            pr_number: None,
        };
        assert!(classify_shaping_result(kind).is_none());
    }

    #[test]
    fn classify_returns_none_for_unrelated_event() {
        let kind = FactoryEventKind::ForgeIdleTick;
        assert!(classify_shaping_result(kind).is_none());
    }
}
