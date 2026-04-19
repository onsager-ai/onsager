//! Built-in heuristic analyzers for Ising v0.1.
//!
//! v0.1 ships with heuristic analyzers, not learned models (ising-v0.1 §6).

pub mod gate_override;
pub mod repeated_failures;
pub mod stuck_artifacts;

pub use gate_override::GateOverrideAnalyzer;
pub use repeated_failures::RepeatedFailuresAnalyzer;
pub use stuck_artifacts::StuckArtifactsAnalyzer;

use crate::core::AnalyzerRegistry;

/// Register all v0.1 built-in analyzers.
pub fn register_defaults(registry: &mut AnalyzerRegistry) {
    registry.register(Box::new(RepeatedFailuresAnalyzer::default()));
    registry.register(Box::new(StuckArtifactsAnalyzer::default()));
    registry.register(Box::new(GateOverrideAnalyzer::default()));
}
