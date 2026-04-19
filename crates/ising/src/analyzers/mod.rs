//! Built-in heuristic analyzers for Ising v0.1.
//!
//! v0.1 ships with heuristic analyzers, not learned models (ising-v0.1 §6).

pub mod gate_deny_rate;
pub mod gate_override;
pub mod pr_churn;
pub mod repeated_failures;
pub mod shape_retry_spike;
pub mod stuck_artifacts;

pub use gate_deny_rate::GateDenyRateAnalyzer;
pub use gate_override::GateOverrideAnalyzer;
pub use pr_churn::PrChurnAnalyzer;
pub use repeated_failures::RepeatedFailuresAnalyzer;
pub use shape_retry_spike::ShapeRetrySpikeAnalyzer;
pub use stuck_artifacts::StuckArtifactsAnalyzer;

use crate::core::AnalyzerRegistry;

/// Register all v0.1 built-in analyzers.
pub fn register_defaults(registry: &mut AnalyzerRegistry) {
    registry.register(Box::new(RepeatedFailuresAnalyzer::default()));
    registry.register(Box::new(StuckArtifactsAnalyzer::default()));
    registry.register(Box::new(GateOverrideAnalyzer::default()));
    registry.register(Box::new(ShapeRetrySpikeAnalyzer::default()));
    registry.register(Box::new(PrChurnAnalyzer::default()));
    registry.register(Box::new(GateDenyRateAnalyzer::default()));
}
