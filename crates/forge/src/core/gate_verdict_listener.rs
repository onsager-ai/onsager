//! Listener that records `synodic.gate_verdict` events into
//! [`PendingVerdicts`] (spec #131 / ADR 0004 Lever C, phase 3).
//!
//! Mirrors the layout of [`crate::core::session_listener`] — filters on
//! `event_type` rather than namespace prefix because the spine listener's
//! namespace filter keys on a `stream_id` prefix that synodic does not
//! currently use for these events.
//!
//! The listener parses each event into the typed [`FactoryEventKind`] and,
//! when it's a `SynodicGateVerdict`, pushes the embedded `GateVerdict` into
//! the shared map under its `gate_id` correlation key. Phase 4 wires the
//! pipeline tick to `take()` that entry on the resume path.

use async_trait::async_trait;
use onsager_spine::factory_event::{FactoryEvent, FactoryEventKind};
use onsager_spine::{EventHandler, EventNotification, EventStore, Listener};

use super::pending::PendingVerdicts;

/// Run the verdict listener forever. Returns only if the underlying
/// pg_notify channel closes.
///
/// `since` is the backfill cursor — forge can persist the last seen event
/// id across restarts so a crash doesn't drop in-flight verdicts. Until
/// phase 6 lands persistence, callers should pass `max_event_id()` so the
/// listener doesn't replay history-wide on every boot.
pub async fn run(
    store: EventStore,
    pending: PendingVerdicts,
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
    pending: PendingVerdicts,
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

/// Pure classifier: extract the `(gate_id, verdict)` pair from a
/// [`FactoryEventKind`], or `None` if the kind is not a gate verdict.
///
/// Split out so tests can exercise the typed branches without a spine.
pub fn classify_verdict(
    kind: FactoryEventKind,
) -> Option<(String, onsager_spine::protocol::GateVerdict)> {
    match kind {
        FactoryEventKind::SynodicGateVerdict {
            gate_id, verdict, ..
        } => Some((gate_id, verdict)),
        _ => None,
    }
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "synodic.gate_verdict" {
            return Ok(());
        }

        let Some(kind) = self.load_kind(&notification).await? else {
            return Ok(());
        };

        if let Some((gate_id, verdict)) = classify_verdict(kind) {
            tracing::info!(
                gate_id = %gate_id,
                "forge: parking synodic.gate_verdict for pipeline resume"
            );
            self.pending.insert(&gate_id, verdict);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::ArtifactId;
    use onsager_spine::factory_event::GatePoint;
    use onsager_spine::protocol::GateVerdict;

    #[test]
    fn classify_extracts_gate_id_and_verdict() {
        let kind = FactoryEventKind::SynodicGateVerdict {
            gate_id: "g_42".into(),
            artifact_id: ArtifactId::new("art_x"),
            gate_point: GatePoint::PreDispatch,
            verdict: GateVerdict::Allow,
        };
        let (gate_id, verdict) = classify_verdict(kind).expect("matches variant");
        assert_eq!(gate_id, "g_42");
        assert!(matches!(verdict, GateVerdict::Allow));
    }

    #[test]
    fn classify_preserves_deny_payload() {
        // Phase-4 will surface the reason on the pipeline event stream;
        // the classifier must not lose it.
        let kind = FactoryEventKind::SynodicGateVerdict {
            gate_id: "g_deny".into(),
            artifact_id: ArtifactId::new("art_x"),
            gate_point: GatePoint::StateTransition,
            verdict: GateVerdict::Deny {
                reason: "policy violation: secrets in payload".into(),
            },
        };
        match classify_verdict(kind).map(|(_, v)| v) {
            Some(GateVerdict::Deny { reason }) => {
                assert_eq!(reason, "policy violation: secrets in payload")
            }
            other => panic!("expected Deny payload preserved, got {other:?}"),
        }
    }

    #[test]
    fn classify_returns_none_for_unrelated_event() {
        // The listener filters by event_type before reaching the
        // classifier, but defense in depth: even if a different variant
        // arrives, we must not park it as a verdict.
        let kind = FactoryEventKind::ForgeIdleTick;
        assert!(classify_verdict(kind).is_none());
    }

    #[test]
    fn classify_returns_none_for_summary_variant() {
        // SynodicGateEvaluated is the dashboard summary; it does not
        // carry the full GateVerdict payload phase-4 will act on.
        let kind = FactoryEventKind::SynodicGateEvaluated {
            gate_id: "g_eval".into(),
            artifact_id: ArtifactId::new("art_x"),
            verdict: onsager_spine::factory_event::VerdictSummary::Allow,
        };
        assert!(classify_verdict(kind).is_none());
    }
}
