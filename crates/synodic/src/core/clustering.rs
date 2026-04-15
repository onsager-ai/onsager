//! Override reason clustering.
//!
//! Groups free-text override reasons into semantic clusters to distinguish
//! "rule is wrong" from "rule is right but context-specific."

use serde::{Deserialize, Serialize};

/// A cluster of semantically similar override reasons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonCluster {
    pub cluster_id: String,
    pub label: String,
    pub suggestion: String,
    pub reasons: Vec<String>,
}

/// Cluster override reasons by keyword matching.
///
/// Returns clusters sorted by count (largest first).
pub fn cluster_reasons(reasons: &[String]) -> Vec<ReasonCluster> {
    let mut non_production = Vec::new();
    let mut expert_override = Vec::new();
    let mut rule_error = Vec::new();
    let mut other = Vec::new();

    for reason in reasons {
        let lower = reason.to_lowercase();
        if is_non_production(&lower) {
            non_production.push(reason.clone());
        } else if is_expert_override(&lower) {
            expert_override.push(reason.clone());
        } else if is_rule_error(&lower) {
            rule_error.push(reason.clone());
        } else {
            other.push(reason.clone());
        }
    }

    let mut clusters = Vec::new();

    if !non_production.is_empty() {
        clusters.push(ReasonCluster {
            cluster_id: "non-production".to_string(),
            label: "Non-production context".to_string(),
            suggestion:
                "Consider adding context-aware exception (e.g., allow on non-protected branches)"
                    .to_string(),
            reasons: non_production,
        });
    }

    if !expert_override.is_empty() {
        clusters.push(ReasonCluster {
            cluster_id: "expert-override".to_string(),
            label: "Expert judgment".to_string(),
            suggestion: "No rule change needed — system working as designed".to_string(),
            reasons: expert_override,
        });
    }

    if !rule_error.is_empty() {
        clusters.push(ReasonCluster {
            cluster_id: "rule-error".to_string(),
            label: "Rule incorrectly flagged safe action".to_string(),
            suggestion: "Flag rule for review — may need narrowing or deprecation".to_string(),
            reasons: rule_error,
        });
    }

    if !other.is_empty() {
        clusters.push(ReasonCluster {
            cluster_id: "other".to_string(),
            label: "Other context".to_string(),
            suggestion: "Manual review recommended — unclear pattern".to_string(),
            reasons: other,
        });
    }

    clusters.sort_by(|a, b| b.reasons.len().cmp(&a.reasons.len()));
    clusters
}

fn is_non_production(s: &str) -> bool {
    let keywords = [
        "test",
        "demo",
        "throwaway",
        "scratch",
        "dev ",
        "development",
        "staging",
        "sandbox",
        "ci branch",
        "ci env",
        "non-prod",
        "feature branch",
        "local",
        "playground",
        "experiment",
    ];
    keywords.iter().any(|k| s.contains(k))
}

fn is_expert_override(s: &str) -> bool {
    let keywords = [
        "know what",
        "intentional",
        "on purpose",
        "deliberate",
        "i understand",
        "i'm aware",
        "expected",
        "by design",
    ];
    keywords.iter().any(|k| s.contains(k))
}

fn is_rule_error(s: &str) -> bool {
    let keywords = [
        "false",
        "not dangerous",
        "wrong",
        "incorrect",
        "bug",
        "false positive",
        "false alarm",
        "safe",
        "harmless",
        "shouldn't block",
        "should not block",
    ];
    keywords.iter().any(|k| s.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_non_production_reasons() {
        let reasons = vec![
            "test environment".to_string(),
            "demo mode".to_string(),
            "throwaway branch".to_string(),
        ];

        let clusters = cluster_reasons(&reasons);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].cluster_id, "non-production");
        assert_eq!(clusters[0].reasons.len(), 3);
    }

    #[test]
    fn cluster_expert_overrides() {
        let reasons = vec![
            "I know what I'm doing".to_string(),
            "intentional force push".to_string(),
        ];

        let clusters = cluster_reasons(&reasons);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].cluster_id, "expert-override");
    }

    #[test]
    fn cluster_rule_errors() {
        let reasons = vec![
            "this is a false positive".to_string(),
            "not dangerous at all".to_string(),
            "this is harmless".to_string(),
        ];

        let clusters = cluster_reasons(&reasons);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].cluster_id, "rule-error");
    }

    #[test]
    fn cluster_mixed_reasons() {
        let reasons = vec![
            "test environment".to_string(),
            "I know what I'm doing".to_string(),
            "false positive".to_string(),
            "just because".to_string(),
        ];

        let clusters = cluster_reasons(&reasons);
        assert_eq!(clusters.len(), 4);
        // Each cluster has 1 reason, sorted by count (all equal, stable order)
    }

    #[test]
    fn empty_reasons_return_empty_clusters() {
        let clusters = cluster_reasons(&[]);
        assert!(clusters.is_empty());
    }

    #[test]
    fn sorted_by_count_descending() {
        let reasons = vec![
            "test env".to_string(),
            "test branch".to_string(),
            "test sandbox".to_string(),
            "false alarm".to_string(),
            "random reason".to_string(),
        ];

        let clusters = cluster_reasons(&reasons);
        assert_eq!(clusters[0].cluster_id, "non-production");
        assert_eq!(clusters[0].reasons.len(), 3);
    }
}
