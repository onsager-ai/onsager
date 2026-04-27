//! Stuck artifacts analyzer (ising-v0.1 §6.2).
//!
//! Looks for artifacts that remain in `in_progress` or `under_review` for too
//! long, or that have been shaped many times without advancing state.
//!
//! Signal: if an artifact has accumulated more than `max_shapings` shaping
//! attempts without reaching a terminal or release state, emit a Waste insight.

use onsager_artifact::ArtifactState;
use onsager_spine::protocol::{FactoryEventRef, Insight};
use onsager_spine::factory_event::{InsightKind, InsightScope};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

/// Configuration for the stuck artifacts analyzer.
#[derive(Debug, Clone)]
pub struct StuckArtifactsConfig {
    /// Maximum shaping attempts before an artifact is considered stuck.
    pub max_shapings: u32,
}

impl Default for StuckArtifactsConfig {
    fn default() -> Self {
        Self { max_shapings: 10 }
    }
}

/// Detects artifacts stuck in non-terminal states with excessive shaping attempts.
pub struct StuckArtifactsAnalyzer {
    config: StuckArtifactsConfig,
}

impl StuckArtifactsAnalyzer {
    pub fn new(config: StuckArtifactsConfig) -> Self {
        Self { config }
    }
}

impl Default for StuckArtifactsAnalyzer {
    fn default() -> Self {
        Self::new(StuckArtifactsConfig::default())
    }
}

impl Analyzer for StuckArtifactsAnalyzer {
    fn name(&self) -> &str {
        "stuck_artifacts"
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        if self.config.max_shapings == 0 {
            return Vec::new();
        }

        let mut insights = Vec::new();

        for tracked in model.artifacts.values() {
            // Only check non-terminal, non-released artifacts.
            if matches!(
                tracked.current_state,
                ArtifactState::Released | ArtifactState::Archived
            ) {
                continue;
            }

            if tracked.shaping_count > self.config.max_shapings {
                // Use real event IDs from shaping history for traceable evidence.
                let history = model.shaping_history(&tracked.artifact_id);
                let evidence: Vec<FactoryEventRef> = history
                    .iter()
                    .rev()
                    .take(3)
                    .map(|record| FactoryEventRef {
                        event_id: record.event_id,
                        event_type: "forge.shaping_returned".into(),
                    })
                    .collect();

                if evidence.is_empty() {
                    continue;
                }

                // Confidence increases with how far over the threshold we are.
                let over_ratio = tracked.shaping_count as f64 / self.config.max_shapings as f64;
                let confidence = (0.5 + (over_ratio - 1.0) * 0.3).min(0.95);

                insights.push(Insight {
                    insight_id: format!(
                        "ins_sa_{}_{}",
                        tracked.artifact_id, model.events_processed
                    ),
                    kind: InsightKind::Waste,
                    scope: InsightScope::SpecificArtifact(tracked.artifact_id.clone()),
                    observation: format!(
                        "artifact {} has been shaped {} times without advancing past {} state",
                        tracked.artifact_id, tracked.shaping_count, tracked.current_state,
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

    fn build_model_with_shapings(count: u32) -> FactoryModel {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_stuck");

        model.ingest(
            1,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind: Kind::Code,
                name: "stuck-service".into(),
                owner: "marvin".into(),
            },
        );
        model.ingest(
            2,
            &FactoryEventKind::ArtifactStateChanged {
                artifact_id: id.clone(),
                from_state: ArtifactState::Draft,
                to_state: ArtifactState::InProgress,
            },
        );

        for i in 0..count {
            model.ingest(
                i as i64 + 3,
                &FactoryEventKind::ForgeShapingReturned {
                    request_id: format!("req_{i}"),
                    artifact_id: id.clone(),
                    outcome: ShapingOutcome::Partial,
                },
            );
        }

        model
    }

    #[test]
    fn detects_stuck_artifact() {
        let model = build_model_with_shapings(15);
        let analyzer = StuckArtifactsAnalyzer::default();
        let insights = analyzer.run(&model);

        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].kind, InsightKind::Waste);
        assert!(insights[0].observation.contains("15 times"));
    }

    #[test]
    fn no_insight_below_threshold() {
        let model = build_model_with_shapings(5);
        let analyzer = StuckArtifactsAnalyzer::default();
        let insights = analyzer.run(&model);

        assert!(insights.is_empty());
    }

    #[test]
    fn skips_released_artifacts() {
        let mut model = build_model_with_shapings(15);
        // Advance to released
        let id = ArtifactId::new("art_stuck");
        model.ingest(
            100,
            &FactoryEventKind::ArtifactStateChanged {
                artifact_id: id,
                from_state: ArtifactState::InProgress,
                to_state: ArtifactState::Released,
            },
        );

        let analyzer = StuckArtifactsAnalyzer::default();
        let insights = analyzer.run(&model);

        assert!(insights.is_empty());
    }

    #[test]
    fn confidence_scales_with_excess() {
        let model_11 = build_model_with_shapings(11);
        let model_20 = build_model_with_shapings(20);
        let analyzer = StuckArtifactsAnalyzer::default();

        let insights_11 = analyzer.run(&model_11);
        let insights_20 = analyzer.run(&model_20);

        assert!(insights_20[0].confidence > insights_11[0].confidence);
    }
}
