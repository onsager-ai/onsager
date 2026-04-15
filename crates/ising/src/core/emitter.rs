//! Insight emitter — validates, deduplicates, and emits insights (ising-v0.1 §7).
//!
//! Insight lifecycle: detected → validated → forwarded → [crystallized]
//!
//! Invariants enforced:
//! - Evidence-backed (invariant #2): insights with empty evidence are rejected
//! - Non-flooding (invariant #8): deduplication within configurable window
//! - Idempotent (invariant #4): same event reprocessed does not produce duplicates

use std::collections::HashSet;

use onsager_spine::protocol::Insight;

/// Configuration for the insight emitter.
#[derive(Debug, Clone)]
pub struct EmitterConfig {
    /// Minimum confidence to record an insight on the spine.
    pub min_confidence: f64,
    /// Confidence threshold for forwarding to Forge as advisory.
    pub advisory_threshold: f64,
    /// Confidence threshold for rule proposal to Synodic.
    pub crystallization_threshold: f64,
}

impl Default for EmitterConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.3,
            advisory_threshold: 0.3,
            crystallization_threshold: 0.7,
        }
    }
}

/// What should happen with a validated insight.
#[derive(Debug, PartialEq, Eq)]
pub enum InsightDisposition {
    /// Record on spine only (below advisory threshold).
    RecordOnly,
    /// Forward to Forge as advisory.
    ForwardToForge,
    /// Forward to Forge AND propose as rule to Synodic.
    ProposeRule,
}

/// Result of attempting to emit an insight.
#[derive(Debug)]
pub enum EmitResult {
    /// Insight was accepted and should be emitted.
    Accepted {
        insight: Insight,
        disposition: InsightDisposition,
    },
    /// Insight was suppressed (dedup or below threshold).
    Suppressed { insight_id: String, reason: String },
    /// Insight was rejected (malformed).
    Rejected { reason: String },
}

/// Validates, deduplicates, and routes insights.
pub struct InsightEmitter {
    config: EmitterConfig,
    /// Recently emitted insight fingerprints for deduplication.
    recent_fingerprints: HashSet<String>,
}

impl InsightEmitter {
    pub fn new(config: EmitterConfig) -> Self {
        Self {
            config,
            recent_fingerprints: HashSet::new(),
        }
    }

    /// Attempt to emit an insight. Returns the disposition.
    pub fn emit(&mut self, insight: Insight) -> EmitResult {
        // Invariant #2: evidence-backed.
        if insight.evidence.is_empty() {
            return EmitResult::Rejected {
                reason: "insight has no evidence (invariant #2 violation)".into(),
            };
        }

        // Minimum confidence check.
        if insight.confidence < self.config.min_confidence {
            return EmitResult::Suppressed {
                insight_id: insight.insight_id,
                reason: format!(
                    "confidence {:.2} below minimum {:.2}",
                    insight.confidence, self.config.min_confidence
                ),
            };
        }

        // Deduplication: fingerprint = kind + scope + observation.
        let fingerprint = format!(
            "{:?}:{:?}:{}",
            insight.kind, insight.scope, insight.observation
        );
        if self.recent_fingerprints.contains(&fingerprint) {
            return EmitResult::Suppressed {
                insight_id: insight.insight_id,
                reason: "duplicate insight within deduplication window".into(),
            };
        }
        self.recent_fingerprints.insert(fingerprint);

        // Route based on confidence.
        let disposition = if insight.confidence >= self.config.crystallization_threshold {
            InsightDisposition::ProposeRule
        } else if insight.confidence >= self.config.advisory_threshold {
            InsightDisposition::ForwardToForge
        } else {
            InsightDisposition::RecordOnly
        };

        EmitResult::Accepted {
            insight,
            disposition,
        }
    }

    /// Clear the deduplication window (e.g., on a new analysis tick).
    pub fn clear_dedup_window(&mut self) {
        self.recent_fingerprints.clear();
    }

    /// Number of fingerprints in the dedup window.
    pub fn dedup_window_size(&self) -> usize {
        self.recent_fingerprints.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::factory_event::{InsightKind, InsightScope};
    use onsager_spine::protocol::FactoryEventRef;

    fn make_insight(id: &str, confidence: f64) -> Insight {
        Insight {
            insight_id: id.into(),
            kind: InsightKind::Failure,
            scope: InsightScope::Global,
            observation: "test pattern detected".into(),
            evidence: vec![FactoryEventRef {
                event_id: 1,
                event_type: "forge.shaping_returned".into(),
            }],
            suggested_action: None,
            confidence,
        }
    }

    fn make_insight_no_evidence(id: &str) -> Insight {
        Insight {
            insight_id: id.into(),
            kind: InsightKind::Failure,
            scope: InsightScope::Global,
            observation: "no evidence".into(),
            evidence: vec![],
            suggested_action: None,
            confidence: 0.9,
        }
    }

    #[test]
    fn rejects_evidenceless_insight() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());
        let result = emitter.emit(make_insight_no_evidence("ins_1"));
        assert!(matches!(result, EmitResult::Rejected { .. }));
    }

    #[test]
    fn suppresses_low_confidence() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());
        let result = emitter.emit(make_insight("ins_1", 0.1));
        assert!(matches!(result, EmitResult::Suppressed { .. }));
    }

    #[test]
    fn accepts_and_routes_to_forge() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());
        let result = emitter.emit(make_insight("ins_1", 0.5));
        match result {
            EmitResult::Accepted { disposition, .. } => {
                assert_eq!(disposition, InsightDisposition::ForwardToForge);
            }
            _ => panic!("expected Accepted"),
        }
    }

    #[test]
    fn high_confidence_proposes_rule() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());
        let result = emitter.emit(make_insight("ins_1", 0.85));
        match result {
            EmitResult::Accepted { disposition, .. } => {
                assert_eq!(disposition, InsightDisposition::ProposeRule);
            }
            _ => panic!("expected Accepted"),
        }
    }

    #[test]
    fn deduplicates_identical_insights() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());

        let result1 = emitter.emit(make_insight("ins_1", 0.5));
        assert!(matches!(result1, EmitResult::Accepted { .. }));

        // Same observation, kind, scope — should be suppressed.
        let result2 = emitter.emit(make_insight("ins_2", 0.5));
        assert!(matches!(result2, EmitResult::Suppressed { .. }));
    }

    #[test]
    fn clear_dedup_allows_re_emit() {
        let mut emitter = InsightEmitter::new(EmitterConfig::default());

        emitter.emit(make_insight("ins_1", 0.5));
        emitter.clear_dedup_window();

        let result = emitter.emit(make_insight("ins_2", 0.5));
        assert!(matches!(result, EmitResult::Accepted { .. }));
    }
}
