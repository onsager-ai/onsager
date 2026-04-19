//! `pr_churn` analyzer (issue #62).
//!
//! Surfaces lineage roots where the factory has opened many PRs without a
//! corresponding merge. The intuition: if the same artifact keeps
//! re-opening PRs the gate or the spec is mis-shaped — the right downstream
//! response is a `PreDispatch` rule that requires (e.g.) passing tests
//! before opening the PR, or a tighter design-review hop.
//!
//! Emission shape: `signal_kind = "pr_churn"`, `subject_ref` is the lineage
//! root (today the PR artifact id), `evidence` is up to five recent
//! `git.pr_opened` event ids, `confidence` scales with how far above the
//! threshold the root sits.

use chrono::Duration;
use onsager_protocol::{FactoryEventRef, Insight};
use onsager_spine::factory_event::{InsightKind, InsightScope};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

/// Stable signal identifier emitted on the spine for this analyzer's output.
pub const SIGNAL_KIND: &str = "pr_churn";

/// Configuration for the PR-churn analyzer.
#[derive(Debug, Clone)]
pub struct PrChurnConfig {
    /// Lookback window over which PR records are considered.
    pub window: Duration,
    /// Minimum number of opened PRs against the same lineage root that
    /// triggers a proposal. Three is conservative — too low produces noise
    /// on small projects, too high never fires on healthy projects.
    pub min_opens: usize,
}

impl Default for PrChurnConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(14),
            min_opens: 3,
        }
    }
}

/// Detects lineage roots with elevated PR-churn (many opens before a merge).
pub struct PrChurnAnalyzer {
    config: PrChurnConfig,
}

impl PrChurnAnalyzer {
    pub fn new(config: PrChurnConfig) -> Self {
        Self { config }
    }
}

impl Default for PrChurnAnalyzer {
    fn default() -> Self {
        Self::new(PrChurnConfig::default())
    }
}

impl Analyzer for PrChurnAnalyzer {
    fn name(&self) -> &str {
        SIGNAL_KIND
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        let activity = model.pr_activity_by_root(self.config.window);
        activity
            .into_iter()
            .filter_map(|(root, (opened, merged, evidence))| {
                if opened < self.config.min_opens {
                    return None;
                }
                // If everything merged, there's no churn; require at least
                // one un-merged open to fire (otherwise a healthy stream of
                // five quick PRs would look like churn).
                if merged >= opened {
                    return None;
                }
                let evidence: Vec<FactoryEventRef> = evidence
                    .into_iter()
                    .map(|event_id| FactoryEventRef {
                        event_id,
                        event_type: "git.pr_opened".into(),
                    })
                    .collect();
                let denom = self.config.min_opens.max(1) as f64;
                let excess = ((opened as f64 - self.config.min_opens as f64) / denom).max(0.0);
                let confidence = (0.6 + excess * 0.3).min(0.95);
                Some(Insight {
                    insight_id: format!("ins_prc_{root}_{opened}"),
                    kind: InsightKind::Waste,
                    scope: InsightScope::ArtifactKind(root.clone()),
                    observation: format!(
                        "{root}: {opened} PR opens vs {merged} merges over the last {} days — \
                         the spec or PreDispatch gate may be too loose, letting under-baked \
                         PRs reach review",
                        self.config.window.num_days(),
                    ),
                    evidence,
                    suggested_action: None,
                    confidence,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_artifact::ArtifactId;
    use onsager_spine::factory_event::FactoryEventKind;

    fn open(model: &mut FactoryModel, id: &ArtifactId, pr: u64, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::GitPrOpened {
                artifact_id: id.clone(),
                repo: "x/y".into(),
                pr_number: pr,
                url: format!("https://example.com/pr/{pr}"),
            },
        );
    }

    fn merge(model: &mut FactoryModel, id: &ArtifactId, pr: u64, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::GitPrMerged {
                artifact_id: id.clone(),
                pr_number: pr,
                merge_sha: "deadbeef".into(),
            },
        );
    }

    #[test]
    fn fires_on_three_opens_no_merge() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_pr_root");
        for i in 0..3 {
            open(&mut model, &id, i + 1, (i as i64) + 1);
        }
        let insights = PrChurnAnalyzer::default().run(&model);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].evidence.len(), 3);
        assert!(insights[0]
            .evidence
            .iter()
            .all(|e| e.event_type == "git.pr_opened"));
    }

    #[test]
    fn does_not_fire_on_one_clean_pr() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_pr_root");
        open(&mut model, &id, 1, 1);
        merge(&mut model, &id, 1, 2);
        assert!(PrChurnAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn does_not_fire_when_all_merged() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_pr_root");
        for i in 0..3 {
            open(&mut model, &id, i + 1, (i as i64) + 1);
            merge(&mut model, &id, i + 1, (i as i64) + 1);
        }
        assert!(PrChurnAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn confidence_climbs_with_open_count() {
        let mut model_low = FactoryModel::new();
        let mut model_high = FactoryModel::new();
        let id = ArtifactId::new("art_pr_root");
        for i in 0..3 {
            open(&mut model_low, &id, i + 1, (i as i64) + 1);
        }
        for i in 0..10 {
            open(&mut model_high, &id, i + 1, (i as i64) + 1);
        }
        let low = &PrChurnAnalyzer::default().run(&model_low)[0];
        let high = &PrChurnAnalyzer::default().run(&model_high)[0];
        assert!(high.confidence >= low.confidence);
    }

    #[test]
    fn analyzer_name_is_signal_kind() {
        assert_eq!(PrChurnAnalyzer::default().name(), SIGNAL_KIND);
    }
}
