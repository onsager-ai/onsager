//! Gate-override-rate analyzer (issue #36 — close the Ising feedback loop).
//!
//! Looks at every Forge `gate.verdict` event over a window (default 7 days)
//! and flags artifact kinds whose deny-plus-escalate ratio crosses a
//! configurable threshold. These are the kinds where policy friction is
//! loudest — prime candidates for rule review, rewording, or retirement.
//!
//! The event stream doesn't yet carry `rule_id`, so grouping is by artifact
//! kind (resolved via `FactoryModel.artifacts`). When gate verdicts start
//! carrying rule identity, this analyzer extends to per-rule grouping
//! without changing its emission contract.
//!
//! Emission shape: `signal_kind = "repeated_gate_override"`, `subject_ref`
//! is the artifact kind (e.g. `"code"`), `evidence` is up to five recent
//! override-verdict event ids, `confidence` scales linearly with the
//! override ratio above the threshold.

use chrono::Duration;
use onsager_spine::protocol::{FactoryEventRef, Insight};
use onsager_spine::factory_event::{InsightKind, InsightScope};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

/// Stable signal identifier emitted on the spine for this analyzer's output.
pub const SIGNAL_KIND: &str = "repeated_gate_override";

/// Configuration for the gate-override analyzer.
#[derive(Debug, Clone)]
pub struct GateOverrideConfig {
    /// Verdict lookback window (default 7 days — matches issue #36 MVP).
    pub window: Duration,
    /// Minimum verdicts observed per kind before a rate is computed.
    pub min_samples: usize,
    /// Override ratio that must be crossed before emitting.
    pub threshold: f64,
}

impl Default for GateOverrideConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_samples: 5,
            threshold: 0.5,
        }
    }
}

/// Detects artifact kinds with a high gate deny/escalate rate.
pub struct GateOverrideAnalyzer {
    config: GateOverrideConfig,
}

impl GateOverrideAnalyzer {
    pub fn new(config: GateOverrideConfig) -> Self {
        Self { config }
    }
}

impl Default for GateOverrideAnalyzer {
    fn default() -> Self {
        Self::new(GateOverrideConfig::default())
    }
}

impl Analyzer for GateOverrideAnalyzer {
    fn name(&self) -> &str {
        SIGNAL_KIND
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        let rates = model.override_rate_by_kind(self.config.window, self.config.min_samples);

        rates
            .into_iter()
            .filter(|(_, (rate, _, _))| *rate >= self.config.threshold)
            .map(|(kind, (rate, total, override_ids))| {
                let kind_label = kind.to_string();
                let evidence: Vec<FactoryEventRef> = override_ids
                    .into_iter()
                    .map(|event_id| FactoryEventRef {
                        event_id,
                        event_type: "forge.gate_verdict".into(),
                    })
                    .collect();

                // Confidence: scale linearly from threshold → 1.0 with headroom
                // so a rate exactly at the threshold still emits with a modest
                // signal (0.5) rather than dropping through `min_confidence`.
                let range = (1.0 - self.config.threshold).max(f64::EPSILON);
                let excess = (rate - self.config.threshold).max(0.0) / range;
                let confidence = (0.5 + excess * 0.4).min(0.95);

                Insight {
                    insight_id: format!("ins_gor_{}_{}", kind_label, model.events_processed),
                    kind: InsightKind::Failure,
                    scope: InsightScope::ArtifactKind(kind_label.clone()),
                    observation: format!(
                        "{} artifacts: {:.0}% gate-override rate over {} verdicts in the \
                         last {} day(s) — rules governing this kind may need review",
                        kind_label,
                        rate * 100.0,
                        total,
                        self.config.window.num_days(),
                    ),
                    evidence,
                    suggested_action: None,
                    confidence,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::{ArtifactId, Kind};
    use onsager_spine::factory_event::{FactoryEventKind, GatePoint, VerdictSummary};

    fn register(model: &mut FactoryModel, id: &ArtifactId, kind: Kind, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind,
                name: "t".into(),
                owner: "marvin".into(),
            },
        );
    }

    fn verdict(model: &mut FactoryModel, id: &ArtifactId, v: VerdictSummary, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::ForgeGateVerdict {
                artifact_id: id.clone(),
                gate_point: GatePoint::PreDispatch,
                verdict: v,
            },
        );
    }

    #[test]
    fn emits_when_override_rate_exceeds_threshold() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_c");
        register(&mut model, &id, Kind::Code, 1);

        // 4 deny + 1 allow = 80% > 50% threshold, 5 samples = min.
        for (i, v) in [
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Deny,
            VerdictSummary::Allow,
        ]
        .iter()
        .enumerate()
        {
            verdict(&mut model, &id, v.clone(), i as i64 + 10);
        }

        let insights = GateOverrideAnalyzer::default().run(&model);
        assert_eq!(insights.len(), 1);
        let ins = &insights[0];
        assert_eq!(ins.kind, InsightKind::Failure);
        assert_eq!(
            ins.scope,
            InsightScope::ArtifactKind("code".into()),
            "scope carries the kind label for downstream routing",
        );
        assert!(!ins.evidence.is_empty());
        assert!(ins.confidence >= 0.5 && ins.confidence <= 0.95);
        assert!(ins.observation.contains("gate-override"));
    }

    #[test]
    fn no_emission_below_threshold() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_c");
        register(&mut model, &id, Kind::Code, 1);

        // 1 deny + 4 allow = 20% < 50% threshold.
        for (i, v) in [
            VerdictSummary::Deny,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
            VerdictSummary::Allow,
        ]
        .iter()
        .enumerate()
        {
            verdict(&mut model, &id, v.clone(), i as i64 + 10);
        }

        assert!(GateOverrideAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn no_emission_when_below_min_samples() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_c");
        register(&mut model, &id, Kind::Code, 1);

        // All denies but only 2 samples — not enough to be confident.
        verdict(&mut model, &id, VerdictSummary::Deny, 10);
        verdict(&mut model, &id, VerdictSummary::Deny, 11);

        assert!(GateOverrideAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn analyzer_name_is_signal_kind() {
        // The Ising emitter uses Analyzer::name() as the signal_kind on the
        // spine event — this test pins the contract so it doesn't silently
        // drift if the analyzer is renamed.
        assert_eq!(GateOverrideAnalyzer::default().name(), SIGNAL_KIND);
    }
}
