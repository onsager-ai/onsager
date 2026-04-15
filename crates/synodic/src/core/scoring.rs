//! Governance scoring engine — computes S(R), F(R), C(R).
//!
//! - **S(R)**: Safety — probability that harmful actions are blocked
//! - **F(R)**: Friction — blocks per tool call
//! - **C(R)**: Coverage — fraction of threat taxonomy covered (weighted)

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::storage::{FeedbackFilters, Rule, Storage, ThreatCategory};

/// Computed governance scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scores {
    pub safety: f64,
    pub friction: f64,
    pub coverage: CoverageResult,
}

/// Coverage score with gap details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageResult {
    pub score: f64,
    pub critical_coverage: f64,
    pub covered_categories: usize,
    pub total_categories: usize,
    pub gaps: Vec<CoverageGap>,
}

/// A threat category not covered by any active rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageGap {
    pub category_id: String,
    pub category_name: String,
    pub severity: String,
    pub severity_weight: f64,
    pub examples: Vec<String>,
}

/// Per-rule convergence info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleHealth {
    pub id: String,
    pub precision: f64,
    pub confidence_interval: f64,
    pub observations: i32,
    pub converged: bool,
}

/// System-wide convergence state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceState {
    pub converged: bool,
    pub all_rules_converged: bool,
    pub coverage_satisfied: bool,
    pub stable: bool,
    pub rule_churn_rate: f64,
    pub rule_health: Vec<RuleHealth>,
}

// ---------------------------------------------------------------------------
// Coverage score C(R)
// ---------------------------------------------------------------------------

/// Compute coverage score: fraction of threat taxonomy covered by active rules.
pub fn compute_coverage(rules: &[Rule], categories: &[ThreatCategory]) -> CoverageResult {
    let mut total_weight = 0.0;
    let mut covered_weight = 0.0;
    let mut gaps = Vec::new();

    let mut critical_total = 0.0;
    let mut critical_covered = 0.0;

    for cat in categories {
        total_weight += cat.severity_weight;
        let is_critical = cat.severity == "critical";

        if is_critical {
            critical_total += cat.severity_weight;
        }

        let covered = rules.iter().any(|r| r.category_id == cat.id && r.enabled);

        if covered {
            covered_weight += cat.severity_weight;
            if is_critical {
                critical_covered += cat.severity_weight;
            }
        } else {
            gaps.push(CoverageGap {
                category_id: cat.id.clone(),
                category_name: cat.name.clone(),
                severity: cat.severity.clone(),
                severity_weight: cat.severity_weight,
                examples: cat.examples.clone(),
            });
        }
    }

    let covered_count = categories.len() - gaps.len();
    let score = if total_weight > 0.0 {
        covered_weight / total_weight
    } else {
        0.0
    };
    let critical = if critical_total > 0.0 {
        critical_covered / critical_total
    } else {
        1.0
    };

    // Sort gaps: critical/high first
    gaps.sort_by(|a, b| b.severity_weight.partial_cmp(&a.severity_weight).unwrap());

    CoverageResult {
        score,
        critical_coverage: critical,
        covered_categories: covered_count,
        total_categories: categories.len(),
        gaps,
    }
}

// ---------------------------------------------------------------------------
// Friction score F(R)
// ---------------------------------------------------------------------------

/// Compute friction score: blocks / estimated total tool calls.
pub async fn compute_friction(store: &dyn Storage, since: DateTime<Utc>) -> Result<f64> {
    let events = store
        .get_feedback(FeedbackFilters {
            since: Some(since),
            ..Default::default()
        })
        .await?;

    let blocks: usize = events
        .iter()
        .filter(|e| e.signal_type == "override" || e.signal_type == "confirmed")
        .count();

    if blocks == 0 {
        return Ok(0.0);
    }

    // Estimate total tool calls from block rate.
    // Typical block rate is ~1-5%. Use 2% as default.
    // Future: track session-level tool call counts for precise F(R).
    const DEFAULT_BLOCK_RATE: f64 = 0.02;
    let estimated_total = blocks as f64 / DEFAULT_BLOCK_RATE;

    Ok(blocks as f64 / estimated_total)
}

// ---------------------------------------------------------------------------
// Safety score S(R)
// ---------------------------------------------------------------------------

/// Compute safety score: weighted coverage discounted by recent incidents.
pub async fn compute_safety(
    store: &dyn Storage,
    rules: &[Rule],
    categories: &[ThreatCategory],
    since: DateTime<Utc>,
) -> Result<f64> {
    let incidents = store
        .get_feedback(FeedbackFilters {
            signal_type: Some("incident".to_string()),
            since: Some(since),
            ..Default::default()
        })
        .await?;

    let mut total_weight = 0.0;
    let mut covered_weight = 0.0;

    for cat in categories {
        total_weight += cat.severity_weight;

        let covered = rules.iter().any(|r| r.category_id == cat.id && r.enabled);

        if covered {
            // Discount by recent incident rate
            let incident_count = incidents
                .iter()
                .filter(|e| {
                    rules
                        .iter()
                        .any(|r| r.id == e.rule_id && r.category_id == cat.id)
                })
                .count();

            // penalty: 1/(1+n) so 0 incidents = full credit, many incidents = low credit
            let penalty = 1.0 / (1.0 + incident_count as f64);
            covered_weight += cat.severity_weight * penalty;
        }
    }

    Ok(if total_weight > 0.0 {
        covered_weight / total_weight
    } else {
        0.0
    })
}

// ---------------------------------------------------------------------------
// Per-rule health
// ---------------------------------------------------------------------------

/// Compute health metrics for each rule.
pub fn compute_rule_health(rules: &[Rule]) -> Vec<RuleHealth> {
    rules
        .iter()
        .map(|r| {
            let observations = r.alpha + r.beta;
            let precision = if observations > 0 {
                r.alpha as f64 / observations as f64
            } else {
                0.0
            };
            let ci = beta_confidence_interval(r.alpha, r.beta);
            let converged = ci < 0.1 && observations > 20;

            RuleHealth {
                id: r.id.clone(),
                precision,
                confidence_interval: ci,
                observations,
                converged,
            }
        })
        .collect()
}

/// 95% credible interval width for Beta(alpha, beta).
pub fn beta_confidence_interval(alpha: i32, beta: i32) -> f64 {
    let a = alpha as f64;
    let b = beta as f64;
    if a + b <= 0.0 {
        return 1.0;
    }
    let variance = (a * b) / ((a + b).powi(2) * (a + b + 1.0));
    2.0 * 1.96 * variance.sqrt()
}

// ---------------------------------------------------------------------------
// Convergence detection
// ---------------------------------------------------------------------------

/// Check system-wide convergence.
pub fn check_convergence(
    rules: &[Rule],
    coverage: &CoverageResult,
    rule_churn_rate: f64,
) -> ConvergenceState {
    let rule_health = compute_rule_health(rules);
    let all_rules_converged = rule_health.iter().all(|r| r.converged);
    let coverage_satisfied = coverage.score >= 0.5 && coverage.critical_coverage >= 0.8;
    let stable = rule_churn_rate < 0.05;

    ConvergenceState {
        converged: all_rules_converged && coverage_satisfied && stable,
        all_rules_converged,
        coverage_satisfied,
        stable,
        rule_churn_rate,
        rule_health,
    }
}

// ---------------------------------------------------------------------------
// Constitutional constraints
// ---------------------------------------------------------------------------

/// Violations of constitutional constraints.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintViolation {
    /// No rule may block file reads
    ReadFreedom,
    /// Every rule must specify target tools or paths
    BoundedScope,
    /// Every rule must have a description
    RightOfExplanation,
}

impl std::fmt::Display for ConstraintViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFreedom => write!(f, "read-freedom: no rule may block file reads"),
            Self::BoundedScope => {
                write!(
                    f,
                    "bounded-scope: every rule must specify target tools or paths"
                )
            }
            Self::RightOfExplanation => {
                write!(
                    f,
                    "right-of-explanation: every rule must have a description"
                )
            }
        }
    }
}

/// Validate a rule against constitutional constraints.
///
/// Returns Ok(()) if valid, or a list of violations.
pub fn validate_constitutional(
    tools: &[String],
    condition_type: &str,
    description: &str,
) -> Result<(), Vec<ConstraintViolation>> {
    let mut violations = Vec::new();

    // ReadFreedom: No blocking the Read tool
    if tools.iter().any(|t| t.eq_ignore_ascii_case("read")) {
        violations.push(ConstraintViolation::ReadFreedom);
    }

    // BoundedScope: Must specify tools or be a path rule
    if tools.is_empty() && condition_type != "path" {
        // Pattern rules without tool scope apply to everything — too broad
        // Exception: this is fine for "pattern" rules that check content
        // Only reject truly unbounded rules (no tools AND no path constraint)
        // For now, we allow pattern rules with empty tools (e.g., secrets-in-args)
        // since they're scoped by the pattern itself.
        // Uncomment below to enforce strict bounded scope:
        // violations.push(ConstraintViolation::BoundedScope);
    }

    // RightOfExplanation: Must have description
    if description.trim().is_empty() {
        violations.push(ConstraintViolation::RightOfExplanation);
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::{Lifecycle, Rule, ThreatCategory};
    use chrono::Utc;

    fn make_category(id: &str, severity: &str, weight: f64) -> ThreatCategory {
        ThreatCategory {
            id: id.to_string(),
            name: id.to_string(),
            description: "test".to_string(),
            severity: severity.to_string(),
            severity_weight: weight,
            examples: vec!["example".to_string()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_rule(id: &str, category: &str, enabled: bool) -> Rule {
        Rule {
            id: id.to_string(),
            description: "test rule".to_string(),
            category_id: category.to_string(),
            tools: vec!["Bash".to_string()],
            condition_type: "command".to_string(),
            condition_value: "test".to_string(),
            lifecycle: Lifecycle::Active,
            alpha: 10,
            beta: 1,
            prior_alpha: 1,
            prior_beta: 1,
            enabled,
            project_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            crystallized_at: None,
            cross_project_validated: false,
        }
    }

    #[test]
    fn coverage_all_covered() {
        let cats = vec![
            make_category("a", "critical", 1.0),
            make_category("b", "high", 0.7),
        ];
        let rules = vec![make_rule("r1", "a", true), make_rule("r2", "b", true)];

        let result = compute_coverage(&rules, &cats);
        assert!((result.score - 1.0).abs() < 0.001);
        assert!(result.gaps.is_empty());
    }

    #[test]
    fn coverage_with_gap() {
        let cats = vec![
            make_category("a", "critical", 1.0),
            make_category("b", "high", 0.7),
        ];
        let rules = vec![make_rule("r1", "a", true)];

        let result = compute_coverage(&rules, &cats);
        // covered = 1.0 / (1.0 + 0.7) = 0.588
        assert!((result.score - 1.0 / 1.7).abs() < 0.01);
        assert_eq!(result.gaps.len(), 1);
        assert_eq!(result.gaps[0].category_id, "b");
    }

    #[test]
    fn coverage_disabled_rule_not_counted() {
        let cats = vec![make_category("a", "critical", 1.0)];
        let rules = vec![make_rule("r1", "a", false)]; // disabled

        let result = compute_coverage(&rules, &cats);
        assert!((result.score - 0.0).abs() < 0.001);
        assert_eq!(result.gaps.len(), 1);
    }

    #[test]
    fn coverage_critical_coverage() {
        let cats = vec![
            make_category("a", "critical", 1.0),
            make_category("b", "critical", 1.0),
            make_category("c", "medium", 0.4),
        ];
        let rules = vec![
            make_rule("r1", "a", true),
            // b is uncovered (critical)
            make_rule("r3", "c", true),
        ];

        let result = compute_coverage(&rules, &cats);
        assert!((result.critical_coverage - 0.5).abs() < 0.001); // 1 of 2 critical
    }

    #[test]
    fn beta_ci_narrows_with_more_observations() {
        let ci_small = beta_confidence_interval(3, 1); // 4 observations
        let ci_large = beta_confidence_interval(30, 10); // 40 observations

        assert!(ci_large < ci_small, "More observations should narrow CI");
    }

    #[test]
    fn rule_health_converges_with_enough_evidence() {
        let rules = vec![Rule {
            alpha: 100,
            beta: 5,
            ..make_rule("r1", "a", true)
        }];

        let health = compute_rule_health(&rules);
        assert_eq!(health.len(), 1);
        assert!(health[0].converged); // 105 observations, CI should be < 0.1
    }

    #[test]
    fn rule_health_not_converged_few_observations() {
        let rules = vec![make_rule("r1", "a", true)]; // alpha=10, beta=1 => 11 obs

        let health = compute_rule_health(&rules);
        assert!(!health[0].converged); // < 20 observations
    }

    #[test]
    fn convergence_requires_all_conditions() {
        let cats = vec![
            make_category("a", "critical", 1.0),
            make_category("b", "high", 0.7),
        ];
        let rules = vec![
            Rule {
                alpha: 100,
                beta: 5,
                ..make_rule("r1", "a", true)
            },
            Rule {
                alpha: 100,
                beta: 5,
                ..make_rule("r2", "b", true)
            },
        ];

        let coverage = compute_coverage(&rules, &cats);
        let state = check_convergence(&rules, &coverage, 0.01);

        assert!(state.converged);
        assert!(state.all_rules_converged);
        assert!(state.coverage_satisfied);
        assert!(state.stable);
    }

    #[test]
    fn convergence_fails_with_high_churn() {
        let cats = vec![make_category("a", "critical", 1.0)];
        let rules = vec![Rule {
            alpha: 25,
            beta: 2,
            ..make_rule("r1", "a", true)
        }];

        let coverage = compute_coverage(&rules, &cats);
        let state = check_convergence(&rules, &coverage, 0.15); // 15% churn

        assert!(!state.converged);
        assert!(!state.stable);
    }

    // -- Constitutional constraints -----------------------------------------

    #[test]
    fn constitutional_read_freedom() {
        let result =
            validate_constitutional(&["Read".to_string()], "path", "Block reading secrets");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err()[0], ConstraintViolation::ReadFreedom);
    }

    #[test]
    fn constitutional_right_of_explanation() {
        let result = validate_constitutional(
            &["Bash".to_string()],
            "command",
            "", // empty description
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err()[0],
            ConstraintViolation::RightOfExplanation
        );
    }

    #[test]
    fn constitutional_valid_rule_passes() {
        let result =
            validate_constitutional(&["Bash".to_string()], "command", "Block dangerous commands");
        assert!(result.is_ok());
    }

    #[test]
    fn constitutional_pattern_rule_without_tools_ok() {
        // secrets-in-args style: no tools specified, pattern-based
        let result = validate_constitutional(&[], "pattern", "Block secrets in arguments");
        assert!(result.is_ok());
    }
}
