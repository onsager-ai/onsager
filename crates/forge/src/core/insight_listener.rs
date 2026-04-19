//! Event-driven listener that tails `ising.insight_emitted` from the spine
//! and pushes parsed insights into the shared [`InsightCache`] (issue #36).
//!
//! Parallel to `session_listener` — filters on the `event_type` string rather
//! than the stream_id namespace so the same wiring catches ising events
//! regardless of how the producer stamps the `stream_id` (`"ising:<subject>"`
//! vs. bare subject).
//!
//! Ising emits the event via `append_ext` with a hand-coded JSON body that
//! matches the `IsingInsightEmitted` variant's field names but without the
//! serde `"type"` tag. This listener therefore reads the fields directly
//! from the raw `data` column rather than deserializing the full envelope.

use std::sync::Arc;

use async_trait::async_trait;
use onsager_protocol::{FactoryEventRef, Insight};
use onsager_spine::{
    EventHandler, EventNotification, EventStore, InsightKind, InsightScope, Listener,
};

use super::insight_cache::InsightCache;

/// Run the ising-insight listener forever. Returns only if the underlying
/// pg_notify channel closes.
pub async fn run(store: EventStore, cache: InsightCache, since: Option<i64>) -> anyhow::Result<()> {
    let dispatcher = Dispatcher {
        store: store.clone(),
        cache: Arc::new(cache),
    };
    Listener::new(store).with_since(since).run(dispatcher).await
}

struct Dispatcher {
    store: EventStore,
    cache: Arc<InsightCache>,
}

#[async_trait]
impl EventHandler for Dispatcher {
    async fn handle(&self, notification: EventNotification) -> anyhow::Result<()> {
        if notification.event_type != "ising.insight_emitted" {
            return Ok(());
        }
        if notification.table != "events_ext" {
            return Ok(());
        }

        let Some(row) = self.store.get_ext_event_by_id(notification.id).await? else {
            return Ok(());
        };

        match parse_insight_from_emitted(row.id, &row.data) {
            Some(insight) => self.cache.push(insight),
            None => tracing::warn!(
                id = row.id,
                "forge: ising.insight_emitted payload failed to parse"
            ),
        }
        Ok(())
    }
}

/// Parse an `ising.insight_emitted` JSON payload into an `Insight` the
/// scheduling kernel understands. We lose the `signal_kind` → kind mapping
/// at this point (the variant uses `Failure` as a conservative default)
/// since nothing in the scheduler branches on kind today; when it does,
/// this mapping moves to a dedicated table.
pub fn parse_insight_from_emitted(event_id: i64, data: &serde_json::Value) -> Option<Insight> {
    let signal_kind = data.get("signal_kind")?.as_str()?;
    let subject_ref = data.get("subject_ref")?.as_str()?;
    let confidence = data.get("confidence")?.as_f64()?;

    let evidence: Vec<FactoryEventRef> = data
        .get("evidence")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    Some(FactoryEventRef {
                        event_id: e.get("event_id")?.as_i64()?,
                        event_type: e.get("event_type")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(Insight {
        insight_id: format!("ins_spine_{event_id}"),
        kind: InsightKind::Failure,
        scope: InsightScope::ArtifactKind(subject_ref.to_string()),
        observation: format!("ising signal: {signal_kind}"),
        evidence,
        suggested_action: None,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_emitted_payload() {
        let data = json!({
            "signal_kind": "repeated_gate_override",
            "subject_ref": "code",
            "evidence": [
                {"event_id": 101, "event_type": "forge.gate_verdict"},
                {"event_id": 98, "event_type": "forge.gate_verdict"},
            ],
            "confidence": 0.82,
        });
        let insight = parse_insight_from_emitted(555, &data).expect("parses");
        assert_eq!(insight.evidence.len(), 2);
        assert_eq!(insight.evidence[0].event_id, 101);
        assert!((insight.confidence - 0.82).abs() < 1e-9);
        assert_eq!(insight.scope, InsightScope::ArtifactKind("code".into()));
        assert!(insight.observation.contains("repeated_gate_override"));
    }

    #[test]
    fn tolerates_missing_evidence_array() {
        let data = json!({
            "signal_kind": "x",
            "subject_ref": "code",
            "confidence": 0.6,
        });
        let insight = parse_insight_from_emitted(1, &data).expect("parses");
        assert!(insight.evidence.is_empty());
    }

    #[test]
    fn rejects_when_required_fields_missing() {
        let data = json!({ "signal_kind": "x" });
        assert!(parse_insight_from_emitted(1, &data).is_none());
    }
}
