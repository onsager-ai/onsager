//! Emission — convert an internal `Insight` into the `ising.insight_emitted`
//! spine event (issue #36) and into `ising.rule_proposed` rule proposal
//! events (issue #36 Step 2).
//!
//! This is deliberately a thin mapping: the analyzer names produce
//! `signal_kind`, the `InsightScope` collapses to a free-form `subject_ref`,
//! and `FactoryEventRef` fields become spine-native `EventRef`s. Kept
//! separate from the emitter so the serve loop can build the event without
//! also running the validation / dedup pipeline.

use onsager_protocol::Insight;
use onsager_spine::factory_event::{
    EventRef, FactoryEventKind, InsightScope, RuleProposalAction, RuleProposalClass,
};

/// Confidence floor above which a `repeated_gate_override` insight becomes
/// a rule-proposal candidate. Below this we stay in the observation-only
/// `insight_emitted` track; at or above it we also emit `rule_proposed`.
/// Conservative — the threshold can only rise, not fall, without an audit
/// of what Synodic auto-activates.
pub const RULE_PROPOSAL_MIN_CONFIDENCE: f64 = 0.80;

/// Confidence floor for `SafeAuto` classification. Below this the proposal
/// still enters the queue but as `ReviewRequired` — a human must click
/// approve before it touches the active rule set.
pub const SAFE_AUTO_MIN_CONFIDENCE: f64 = 0.90;

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

/// Build an `ising.rule_proposed` variant from an insight when the signal
/// warrants a rule change. Returns `None` when the signal kind has no
/// rule-proposal mapping or the confidence is below
/// [`RULE_PROPOSAL_MIN_CONFIDENCE`].
///
/// The only signal wired in this step is `repeated_gate_override`, which
/// maps to a `Retire` proposal: when a rule fires and is overridden more
/// often than it's respected, the policy is costing more than it buys.
/// Future signals (shape-retry spike, cross-project divergence) add their
/// own arm to the match below without changing the event contract.
pub fn insight_to_rule_proposal(signal_kind: &str, insight: &Insight) -> Option<FactoryEventKind> {
    if insight.confidence < RULE_PROPOSAL_MIN_CONFIDENCE {
        return None;
    }

    let subject_ref = subject_ref_from_scope(&insight.scope);
    let proposed_action = match signal_kind {
        "repeated_gate_override" => RuleProposalAction::Retire {
            // The insight currently groups by artifact kind rather than by
            // rule id (§gate_override.rs); the Synodic consumer maps the
            // kind back to the ruling rule via the feedback_events table.
            // Using the subject_ref as rule_id is a deliberate placeholder
            // that keeps the proposal self-contained — Synodic resolves it
            // at queue time rather than the producer embedding a join.
            rule_id: subject_ref.clone(),
        },
        _ => return None,
    };

    let class = if insight.confidence >= SAFE_AUTO_MIN_CONFIDENCE {
        RuleProposalClass::SafeAuto
    } else {
        RuleProposalClass::ReviewRequired
    };

    Some(FactoryEventKind::IsingRuleProposed {
        insight_id: insight.insight_id.clone(),
        signal_kind: signal_kind.to_owned(),
        subject_ref,
        proposed_action,
        class,
        rationale: insight.observation.clone(),
        confidence: insight.confidence,
    })
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

    fn insight_with_conf(conf: f64) -> Insight {
        let mut i = make_insight(InsightScope::ArtifactKind("code".into()));
        i.confidence = conf;
        i
    }

    #[test]
    fn rule_proposal_skipped_when_confidence_below_threshold() {
        // Below RULE_PROPOSAL_MIN_CONFIDENCE the insight stays on the
        // observation track only — no rule proposal leaks onto the spine.
        let proposal = insight_to_rule_proposal("repeated_gate_override", &insight_with_conf(0.65));
        assert!(proposal.is_none(), "0.65 is below threshold, must skip");
    }

    #[test]
    fn rule_proposal_uses_review_required_in_between_thresholds() {
        let proposal = insight_to_rule_proposal("repeated_gate_override", &insight_with_conf(0.82))
            .expect("high enough to propose");
        match proposal {
            FactoryEventKind::IsingRuleProposed {
                class,
                proposed_action,
                subject_ref,
                signal_kind,
                ..
            } => {
                assert_eq!(class, RuleProposalClass::ReviewRequired);
                assert_eq!(signal_kind, "repeated_gate_override");
                assert_eq!(subject_ref, "code");
                match proposed_action {
                    RuleProposalAction::Retire { rule_id } => assert_eq!(rule_id, "code"),
                    other => panic!("expected Retire, got {other:?}"),
                }
            }
            _ => panic!("expected IsingRuleProposed"),
        }
    }

    #[test]
    fn rule_proposal_is_safe_auto_at_high_confidence() {
        let proposal = insight_to_rule_proposal("repeated_gate_override", &insight_with_conf(0.95))
            .expect("proposal emitted");
        match proposal {
            FactoryEventKind::IsingRuleProposed { class, .. } => {
                assert_eq!(class, RuleProposalClass::SafeAuto);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn unknown_signal_kind_does_not_propose() {
        // Guardrail: a new analyzer must explicitly opt in to rule-proposal
        // routing. Silent passthrough would let a noisy signal auto-mutate
        // rules without an author thinking about the mapping.
        let proposal = insight_to_rule_proposal("totally_new_signal", &insight_with_conf(0.99));
        assert!(proposal.is_none());
    }
}
