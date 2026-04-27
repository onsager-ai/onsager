//! Repeated shaping failures analyzer (ising-v0.1 §6.1).
//!
//! Looks for artifacts or artifact kinds where shaping requests consistently
//! return Failed or Aborted outcomes.
//!
//! Signal: if more than N of the last M shaping requests for a given scope
//! have failed, emit a Failure insight. Default: N=3, M=5.

use onsager_spine::factory_event::{InsightKind, InsightScope};
use onsager_spine::protocol::{FactoryEventRef, Insight};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

/// Configuration for the repeated failures analyzer.
#[derive(Debug, Clone)]
pub struct RepeatedFailuresConfig {
    /// Minimum failures in the window to trigger an insight.
    pub min_failures: usize,
    /// Window size (last M shaping attempts).
    pub window_size: usize,
}

impl Default for RepeatedFailuresConfig {
    fn default() -> Self {
        Self {
            min_failures: 3,
            window_size: 5,
        }
    }
}

/// Detects artifacts with repeated shaping failures.
pub struct RepeatedFailuresAnalyzer {
    config: RepeatedFailuresConfig,
}

impl RepeatedFailuresAnalyzer {
    pub fn new(config: RepeatedFailuresConfig) -> Self {
        Self { config }
    }
}

impl Default for RepeatedFailuresAnalyzer {
    fn default() -> Self {
        Self::new(RepeatedFailuresConfig::default())
    }
}

impl Analyzer for RepeatedFailuresAnalyzer {
    fn name(&self) -> &str {
        "repeated_failures"
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        if self.config.window_size == 0 {
            return Vec::new();
        }

        let mut insights = Vec::new();

        for tracked in model.artifacts.values() {
            let failure_rate = model.failure_rate(&tracked.artifact_id, self.config.window_size);
            let threshold = self.config.min_failures as f64 / self.config.window_size as f64;

            if failure_rate >= threshold {
                let history = model.shaping_history(&tracked.artifact_id);
                let evidence: Vec<FactoryEventRef> = history
                    .iter()
                    .rev()
                    .take(self.config.window_size)
                    .map(|record| FactoryEventRef {
                        event_id: record.event_id,
                        event_type: "forge.shaping_returned".into(),
                    })
                    .collect();

                if evidence.is_empty() {
                    continue;
                }

                // Confidence scales with failure rate.
                let confidence = (failure_rate * 0.9).min(0.95);

                insights.push(Insight {
                    insight_id: format!(
                        "ins_rf_{}_{}",
                        tracked.artifact_id, model.events_processed
                    ),
                    kind: InsightKind::Failure,
                    scope: InsightScope::SpecificArtifact(tracked.artifact_id.clone()),
                    observation: format!(
                        "artifact {} has {:.0}% failure rate over last {} shaping attempts",
                        tracked.artifact_id,
                        failure_rate * 100.0,
                        self.config.window_size,
                    ),
                    evidence,
                    suggested_action: None,
                    confidence,
                });
            }
        }

        insights
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

    fn build_model_with_failures(completed: u32, failed: u32) -> FactoryModel {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_test1");

        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "test".into(),
                owner: "marvin".into(),
            },
        );

        let mut seq = 2i64;
        for _ in 0..completed {
            model.ingest(
                seq,
                &FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_{seq}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Completed,
                },
            );
            seq += 1;
        }
        for _ in 0..failed {
            model.ingest(
                seq,
                &FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_{seq}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Failed,
                },
            );
            seq += 1;
        }

        model
    }

    #[test]
    fn detects_repeated_failures() {
        // 1 completed + 4 failed = 80% failure in last 5
        let model = build_model_with_failures(1, 4);
        let analyzer = RepeatedFailuresAnalyzer::default();
        let insights = analyzer.run(&model);

        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].kind, InsightKind::Failure);
        assert!(insights[0].confidence > 0.5);
    }

    #[test]
    fn no_insight_for_healthy_artifact() {
        // 5 completed + 0 failed
        let model = build_model_with_failures(5, 0);
        let analyzer = RepeatedFailuresAnalyzer::default();
        let insights = analyzer.run(&model);

        assert!(insights.is_empty());
    }

    #[test]
    fn threshold_boundary() {
        // 2 completed + 3 failed = 60% failure (exactly at 3/5 threshold)
        let model = build_model_with_failures(2, 3);
        let analyzer = RepeatedFailuresAnalyzer::default();
        let insights = analyzer.run(&model);

        assert_eq!(insights.len(), 1);
    }

    #[test]
    fn below_threshold() {
        // 3 completed + 2 failed = 40% failure (below 3/5 threshold)
        let model = build_model_with_failures(3, 2);
        let analyzer = RepeatedFailuresAnalyzer::default();
        let insights = analyzer.run(&model);

        assert!(insights.is_empty());
    }
}
