//! `gate_deny_rate` analyzer (issue #62).
//!
//! Sister of `gate_override`, but inverted: when *forge*-emitted Deny
//! verdicts dominate within a rolling window, the rule producing them is a
//! candidate for *Rewrite* (relax the condition) rather than `Retire`
//! (drop entirely). Keeps Phase 3 honest about runaway gates that block
//! every PR — a healthy factory has both Allow and Deny outcomes.

use chrono::Duration;
use onsager_artifact::Kind;
use onsager_spine::factory_event::{InsightKind, InsightScope, VerdictSummary};
use onsager_spine::protocol::{FactoryEventRef, Insight};

use crate::core::analyzer::Analyzer;
use crate::core::model::FactoryModel;

pub const SIGNAL_KIND: &str = "gate_deny_rate";

#[derive(Debug, Clone)]
pub struct GateDenyRateConfig {
    pub window: Duration,
    /// Minimum verdicts in the window before a rate is computed.
    pub min_samples: usize,
    /// Deny rate at or above which the kind is flagged. Default 0.40 mirrors
    /// the spec's worked example (`>40% over last 20 PRs`).
    pub min_deny_rate: f64,
}

impl Default for GateDenyRateConfig {
    fn default() -> Self {
        Self {
            window: Duration::days(7),
            min_samples: 20,
            min_deny_rate: 0.40,
        }
    }
}

pub struct GateDenyRateAnalyzer {
    config: GateDenyRateConfig,
}

impl GateDenyRateAnalyzer {
    pub fn new(config: GateDenyRateConfig) -> Self {
        Self { config }
    }
}

impl Default for GateDenyRateAnalyzer {
    fn default() -> Self {
        Self::new(GateDenyRateConfig::default())
    }
}

impl Analyzer for GateDenyRateAnalyzer {
    fn name(&self) -> &str {
        SIGNAL_KIND
    }

    fn run(&self, model: &FactoryModel) -> Vec<Insight> {
        let cutoff = chrono::Utc::now() - self.config.window;
        let mut by_kind: std::collections::HashMap<Kind, Vec<i64>> = Default::default();
        let mut totals: std::collections::HashMap<Kind, usize> = Default::default();
        let mut deny_ids: std::collections::HashMap<Kind, Vec<i64>> = Default::default();

        for r in model
            .gate_verdict_records
            .iter()
            .filter(|r| r.recorded_at >= cutoff)
        {
            let Some(art) = model.artifacts.get(r.artifact_id.as_str()) else {
                continue;
            };
            *totals.entry(art.kind.clone()).or_default() += 1;
            by_kind
                .entry(art.kind.clone())
                .or_default()
                .push(r.event_id);
            if matches!(r.verdict, VerdictSummary::Deny) {
                deny_ids
                    .entry(art.kind.clone())
                    .or_default()
                    .push(r.event_id);
            }
        }

        totals
            .into_iter()
            .filter_map(|(kind, total)| {
                if total < self.config.min_samples {
                    return None;
                }
                let denies = deny_ids.get(&kind).map(|v| v.len()).unwrap_or(0);
                let rate = denies as f64 / total as f64;
                if rate < self.config.min_deny_rate {
                    return None;
                }
                let mut evidence_ids = deny_ids.get(&kind).cloned().unwrap_or_default();
                evidence_ids.sort_unstable_by(|a, b| b.cmp(a));
                evidence_ids.truncate(5);
                let evidence: Vec<FactoryEventRef> = evidence_ids
                    .into_iter()
                    .map(|event_id| FactoryEventRef {
                        event_id,
                        event_type: "forge.gate_verdict".into(),
                    })
                    .collect();
                let kind_label = kind.to_string();
                let denom = self.config.min_deny_rate.max(0.01);
                let excess = ((rate - self.config.min_deny_rate) / denom).max(0.0);
                let confidence = (0.6 + excess * 0.3).min(0.95);
                Some(Insight {
                    insight_id: format!("ins_gdr_{kind_label}_{denies}_{total}"),
                    kind: InsightKind::Failure,
                    scope: InsightScope::ArtifactKind(kind_label.clone()),
                    observation: format!(
                        "{kind_label}: {denies}/{total} ({:.0}%) verdicts denied over the last \
                         {} day(s) — the gate may be too strict; review the rule before it \
                         becomes friction overhead",
                        rate * 100.0,
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
    use onsager_spine::factory_event::{FactoryEventKind, GatePoint};

    fn register(model: &mut FactoryModel, id: &ArtifactId, kind: Kind, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::ArtifactRegistered {
                artifact_id: id.clone(),
                kind,
                name: "x".into(),
                owner: "marvin".into(),
            },
        );
    }

    fn verdict(model: &mut FactoryModel, id: &ArtifactId, v: VerdictSummary, seq: i64) {
        model.ingest(
            seq,
            &FactoryEventKind::ForgeGateVerdict {
                artifact_id: id.clone(),
                gate_point: GatePoint::StateTransition,
                verdict: v,
            },
        );
    }

    #[test]
    fn fires_when_deny_rate_exceeds_threshold() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_code");
        register(&mut model, &id, Kind::Code, 1);
        for i in 0..10 {
            verdict(&mut model, &id, VerdictSummary::Deny, i + 2);
        }
        for i in 0..10 {
            verdict(&mut model, &id, VerdictSummary::Allow, i + 12);
        }
        // 10/20 = 50% > 40% threshold, exactly 20 samples = at threshold.
        let insights = GateDenyRateAnalyzer::default().run(&model);
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].evidence.len(), 5);
        assert!(insights[0]
            .evidence
            .iter()
            .all(|e| e.event_type == "forge.gate_verdict"));
    }

    #[test]
    fn does_not_fire_below_threshold() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_code");
        register(&mut model, &id, Kind::Code, 1);
        // 4 denies / 20 total = 20% — under 40%.
        for i in 0..4 {
            verdict(&mut model, &id, VerdictSummary::Deny, i + 2);
        }
        for i in 0..16 {
            verdict(&mut model, &id, VerdictSummary::Allow, i + 6);
        }
        assert!(GateDenyRateAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn does_not_fire_below_min_samples() {
        let mut model = FactoryModel::new();
        let id = ArtifactId::new("art_code");
        register(&mut model, &id, Kind::Code, 1);
        for i in 0..5 {
            verdict(&mut model, &id, VerdictSummary::Deny, i + 2);
        }
        // 5 denies, 5 total — far above threshold rate but below min_samples.
        assert!(GateDenyRateAnalyzer::default().run(&model).is_empty());
    }

    #[test]
    fn analyzer_name_is_signal_kind() {
        assert_eq!(GateDenyRateAnalyzer::default().name(), SIGNAL_KIND);
    }
}
