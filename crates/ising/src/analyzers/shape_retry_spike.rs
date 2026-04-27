//! Shape-retry-spike analyzer (issue #36 follow-up — second wired signal).
//!
//! Looks at the average shaping attempts per artifact within a window, grouped
//! by artifact `Kind`. A kind whose artifacts systematically need many reshape
//! cycles is a kind whose decomposer or shaping rules are underspecified —
//! the right downstream response is a Synodic rule that caps rework or tightens
//! the spec, hence the `Introduce`-action rule proposal.
//!
//! Emission shape: `signal_kind = "shape_retry_spike"`, `subject_ref` is the
//! artifact kind, `evidence` is up to five recent `forge.shaping_returned`
//! event ids, `confidence` scales with how far above the per-artifact retry
//! threshold the kind sits.

use chrono::Duration;
use onsager_spine::protocol::{FactoryEventRef, Insight};
use onsager_spine::factory_event::{InsightKind, InsightScope};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

/// Stable signal identifier emitted on the spine for this analyzer's output.
pub const SIGNAL_KIND: &str = "shape_retry_spike";

/// Configuration for the shape-retry-spike analyzer.
#[derive(Debug, Clone)]
pub struct ShapeRetrySpikeConfig {
    /// Lookback window over which shaping records are considered.
    pub window: Duration,
    /// Minimum distinct artifacts of a kind required before a rate is computed.
    pub min_artifacts: usize,
    /// Average shaping attempts per artifact above which the kind is flagged.
    pub min_avg_shapings: f64,
}

impl Default for ShapeRetrySpikeConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_artifacts: 3,
            // 4 reshapes/artifact on average is generous — Forge's default
            // budget is small, so anything north of this is real friction.
            min_avg_shapings: 4.0,
        }
    }
}

/// Detects artifact kinds with elevated shaping-retry averages.
pub struct ShapeRetrySpikeAnalyzer {
    config: ShapeRetrySpikeConfig,
}

impl ShapeRetrySpikeAnalyzer {
    pub fn new(config: ShapeRetrySpikeConfig) -> Self {
        Self { config }
    }
}

impl Default for ShapeRetrySpikeAnalyzer {
    fn default() -> Self {
        Self::new(ShapeRetrySpikeConfig::default())
    }
}

impl Analyzer for ShapeRetrySpikeAnalyzer {
    fn name(&self) -> &str {
        SIGNAL_KIND
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        let spikes = model.retry_spike_by_kind(self.config.window, self.config.min_artifacts);

        spikes
            .into_iter()
            .filter(|(_, (avg, _, _))| *avg >= self.config.min_avg_shapings)
            .map(|(kind, (avg, artifact_count, evidence_ids))| {
                let kind_label = kind.to_string();
                let evidence: Vec<FactoryEventRef> = evidence_ids
                    .into_iter()
                    .map(|event_id| FactoryEventRef {
                        event_id,
                        event_type: "forge.shaping_returned".into(),
                    })
                    .collect();

                // Confidence: 0.6 at the threshold, climbing toward 0.95 as
                // the kind drifts further above it. Mirrors the gate-override
                // analyzer's shape so dashboard sorting stays comparable.
                let denom = self.config.min_avg_shapings.max(1.0);
                let excess = ((avg - self.config.min_avg_shapings) / denom).max(0.0);
                let confidence = (0.6 + excess * 0.3).min(0.95);

                Insight {
                    insight_id: format!("ins_srs_{}_{}", kind_label, model.events_processed),
                    kind: InsightKind::Waste,
                    scope: InsightScope::ArtifactKind(kind_label.clone()),
                    observation: format!(
                        "{} artifacts: avg {:.1} shaping attempts per artifact across {} \
                         artifacts in the last {} day(s) — decomposition or shaping rules \
                         for this kind may be underspecified",
                        kind_label,
                        avg,
                        artifact_count,
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
    use onsager_spine::factory_event::{FactoryEventKind, ShapingOutcome};

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

    fn shape(model: &mut FactoryModel, id: &ArtifactId, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::ForgeShapingReturned {
                request_id: format!("req_{seq}"),
                artifact_id: id.clone(),
                outcome: ShapingOutcome::Partial,
            },
        );
    }

    #[test]
    fn emits_when_avg_shapings_exceeds_threshold() {
        // 3 code artifacts, each with 5 shapings = avg 5.0 (> default 4.0).
        let mut model = FactoryModel::new();
        let mut seq = 1i64;
        for n in 0..3 {
            let id = ArtifactId::new(format!("art_code_{n}"));
            register(&mut model, &id, Kind::Code, seq);
            seq += 1;
            for _ in 0..5 {
                shape(&mut model, &id, seq);
                seq += 1;
            }
        }

        let insights = ShapeRetrySpikeAnalyzer::default().run(&model);
        assert_eq!(insights.len(), 1);
        let ins = &insights[0];
        assert_eq!(ins.kind, InsightKind::Waste);
        assert_eq!(
            ins.scope,
            InsightScope::ArtifactKind("code".into()),
            "scope must carry the kind label for downstream rule routing",
        );
        assert!(!ins.evidence.is_empty());
        assert!(ins.evidence.len() <= 5, "evidence is capped at 5");
        assert!(
            ins.evidence
                .iter()
                .all(|e| e.event_type == "forge.shaping_returned"),
            "evidence must reference shaping events",
        );
        assert!(ins.confidence >= 0.6 && ins.confidence <= 0.95);
        assert!(ins.observation.contains("shaping attempts"));
    }

    #[test]
    fn no_emission_below_avg_threshold() {
        // 3 code artifacts, each with 2 shapings = avg 2.0 (< default 4.0).
        let mut model = FactoryModel::new();
        let mut seq = 1i64;
        for n in 0..3 {
            let id = ArtifactId::new(format!("art_code_{n}"));
            register(&mut model, &id, Kind::Code, seq);
            seq += 1;
            for _ in 0..2 {
                shape(&mut model, &id, seq);
                seq += 1;
            }
        }
        assert!(ShapeRetrySpikeAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn no_emission_below_min_artifacts() {
        // 1 code artifact with 20 shapings — high rework but only 1 artifact,
        // so it's a `stuck_artifacts` story, not a kind-wide spike.
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_code_0");
        register(&mut model, &id, Kind::Code, 1);
        for i in 0..20 {
            shape(&mut model, &id, i + 2);
        }
        assert!(ShapeRetrySpikeAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn analyzer_name_is_signal_kind() {
        // The Ising emitter uses Analyzer::name() as the signal_kind on the
        // spine event — pin the contract so renames don't silently drift.
        assert_eq!(ShapeRetrySpikeAnalyzer::default().name(), SIGNAL_KIND);
    }

    #[test]
    fn confidence_climbs_with_avg() {
        let make = |shapings_per_artifact: usize| {
            let mut model = FactoryModel::new();
            let mut seq = 1i64;
            for n in 0..3 {
                let id = ArtifactId::new(format!("art_code_{n}"));
                register(&mut model, &id, Kind::Code, seq);
                seq += 1;
                for _ in 0..shapings_per_artifact {
                    shape(&mut model, &id, seq);
                    seq += 1;
                }
            }
            model
        };

        let low = ShapeRetrySpikeAnalyzer::default().run(&make(5));
        let high = ShapeRetrySpikeAnalyzer::default().run(&make(15));
        assert!(!low.is_empty() && !high.is_empty());
        assert!(
            high[0].confidence >= low[0].confidence,
            "more rework must not reduce confidence",
        );
    }
}
