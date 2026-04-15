//! Core Ising logic — analyzer trait, factory model, insight pipeline.

pub mod analyzer;
pub mod emitter;
pub mod model;

pub use analyzer::{Analyzer, AnalyzerRegistry};
pub use emitter::InsightEmitter;
pub use model::FactoryModel;
