//! Core Ising logic — analyzer trait, factory model, insight pipeline.

pub mod analyzer;
pub mod emission;
pub mod emitter;
pub mod model;

pub use analyzer::{Analyzer, AnalyzerRegistry};
pub use emission::insight_to_emitted_event;
pub use emitter::InsightEmitter;
pub use model::FactoryModel;
