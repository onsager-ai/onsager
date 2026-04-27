//! Analyzer trait and registry (ising-v0.1 §6.8).
//!
//! The analyzer contract:
//! ```text
//! trait Analyzer:
//!     fn name(&self) -> &str
//!     fn run(&self, model: &FactoryModel) -> Vec<Insight>
//! ```
//!
//! Any implementation that produces well-formed insights (with evidence,
//! valid confidence, correct scope) is valid.

use onsager_spine::protocol::Insight;

use super::model::FactoryModel;

/// Pluggable analyzer contract (ising-v0.1 §6.8).
pub trait Analyzer: Send + Sync {
    /// Human-readable analyzer name.
    fn name(&self) -> &str;

    /// Run analysis over the accumulated factory model and produce insights.
    fn run(&self, model: &FactoryModel) -> Vec<Insight>;
}

/// Registry of active analyzers.
#[derive(Default)]
pub struct AnalyzerRegistry {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new analyzer.
    pub fn register(&mut self, analyzer: Box<dyn Analyzer>) {
        self.analyzers.push(analyzer);
    }

    /// Run all registered analyzers and collect their insights.
    pub fn run_all(&self, model: &FactoryModel) -> Vec<(String, Vec<Insight>)> {
        self.analyzers
            .iter()
            .map(|a| {
                let insights = a.run(model);
                (a.name().to_owned(), insights)
            })
            .collect()
    }

    /// Number of registered analyzers.
    pub fn len(&self) -> usize {
        self.analyzers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.analyzers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use onsager_spine::protocol::FactoryEventRef;
    use onsager_spine::factory_event::{InsightKind, InsightScope};

    struct DummyAnalyzer;
    impl Analyzer for DummyAnalyzer {
        fn name(&self) -> &str {
            "dummy"
        }
        fn run(&self, _model: &FactoryModel) -> Vec<Insight> {
            vec![Insight {
                insight_id: "ins_dummy".into(),
                kind: InsightKind::Win,
                scope: InsightScope::Global,
                observation: "everything is great".into(),
                evidence: vec![FactoryEventRef {
                    event_id: 1,
                    event_type: "forge.idle_tick".into(),
                }],
                suggested_action: None,
                confidence: 0.5,
            }]
        }
    }

    #[test]
    fn registry_runs_all_analyzers() {
        let mut registry = AnalyzerRegistry::new();
        registry.register(Box::new(DummyAnalyzer));
        assert_eq!(registry.len(), 1);

        let model = FactoryModel::new();
        let results = registry.run_all(&model);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "dummy");
        assert_eq!(results[0].1.len(), 1);
    }

    #[test]
    fn empty_registry() {
        let registry = AnalyzerRegistry::new();
        assert!(registry.is_empty());
        let results = registry.run_all(&FactoryModel::new());
        assert!(results.is_empty());
    }
}
