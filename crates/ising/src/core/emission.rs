//! Emission — convert an internal `Insight` into the `ising.insight_emitted`
//! spine event (issue #36).
//!
//! This is deliberately a thin mapping: the analyzer names produce
//! `signal_kind`, the `InsightScope` collapses to a free-form `subject_ref`,
//! and `FactoryEventRef` fields become spine-native `EventRef`s. Kept
//! separate from the emitter so the serve loop can build the event without
//! also running the validation / dedup pipeline.

use onsager_protocol::Insight;
use onsager_spine::factory_event::{EventRef, FactoryEventKind, InsightScope};

/// Build the `ising.insight_emitted` variant from an accepted `Insight` and
/// the name of the analyzer that produced it.
pub fn insight_to_emitted_event(signal_kind: &str, insight: &Insight) -> FactoryEventKind {
    let subject_ref = subject_ref_from_scope(&insight.scope);
    let evidence = insight
        .evidence
        .iter()
        .map(|e| EventRef {
            event_id: e.event_id,
            event_type: e.event_type.clone(),
        })
        .collect();

    FactoryEventKind::IsingInsightEmitted {
        signal_kind: signal_kind.to_owned(),
        subject_ref,
        evidence,
        confidence: insight.confidence,
    }
}

/// Collapse an `InsightScope` to a `subject_ref` string — the identifier a
/// downstream consumer joins on.
fn subject_ref_from_scope(scope: &InsightScope) -> String {
    match scope {
        InsightScope::Global => "global".to_string(),
        InsightScope::ArtifactKind(k) => k.clone(),
        InsightScope::SpecificArtifact(id) => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::ArtifactId;
    use onsager_protocol::FactoryEventRef;
    use onsager_spine::factory_event::InsightKind;

    fn make_insight(scope: InsightScope) -> Insight {
        Insight {
            insight_id: "ins_1".into(),
            kind: InsightKind::Failure,
            scope,
            observation: "o".into(),
            evidence: vec![FactoryEventRef {
                event_id: 7,
                event_type: "forge.gate_verdict".into(),
            }],
            suggested_action: None,
            confidence: 0.73,
        }
    }

    #[test]
    fn maps_artifact_kind_scope() {
        let evt = insight_to_emitted_event(
            "repeated_gate_override",
            &make_insight(InsightScope::ArtifactKind("code".into())),
        );
        match evt {
            FactoryEventKind::IsingInsightEmitted {
                signal_kind,
                subject_ref,
                evidence,
                confidence,
            } => {
                assert_eq!(signal_kind, "repeated_gate_override");
                assert_eq!(subject_ref, "code");
                assert_eq!(evidence.len(), 1);
                assert_eq!(evidence[0].event_id, 7);
                assert!((confidence - 0.73).abs() < 1e-9);
            }
            _ => panic!("expected IsingInsightEmitted"),
        }
    }

    #[test]
    fn maps_specific_artifact_scope_to_artifact_id() {
        let evt = insight_to_emitted_event(
            "shape_retry_spike",
            &make_insight(InsightScope::SpecificArtifact(ArtifactId::new("art_abc"))),
        );
        match evt {
            FactoryEventKind::IsingInsightEmitted { subject_ref, .. } => {
                assert_eq!(subject_ref, "art_abc");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn maps_global_scope() {
        let evt = insight_to_emitted_event("x", &make_insight(InsightScope::Global));
        match evt {
            FactoryEventKind::IsingInsightEmitted { subject_ref, .. } => {
                assert_eq!(subject_ref, "global");
            }
            _ => panic!(),
        }
    }
}
