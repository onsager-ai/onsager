use crate::core::{scoring, storage};
use anyhow::Result;
use chrono::Duration;
use clap::Args;

/// Show governance status: safety, friction, coverage scores.
#[derive(Args)]
pub struct StatusCmd {
    /// Database URL (defaults to ~/.synodic/synodic.db)
    #[arg(long, env = "DATABASE_URL")]
    db_url: Option<String>,

    /// Lookback period in days for friction/safety (default: 30)
    #[arg(long, default_value = "30")]
    days: i64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

impl StatusCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = self
            .db_url
            .unwrap_or_else(storage::pool::resolve_database_url);
        let store = storage::pool::create_storage(&db_url).await?;

        let rules = store.get_rules(true).await?;
        let categories = store.get_threat_categories().await?;
        let since = chrono::Utc::now() - Duration::days(self.days);

        // Compute scores
        let coverage = scoring::compute_coverage(&rules, &categories);
        let safety = scoring::compute_safety(&*store, &rules, &categories, since).await?;
        let friction = scoring::compute_friction(&*store, since).await?;
        let rule_health = scoring::compute_rule_health(&rules);
        let convergence = scoring::check_convergence(&rules, &coverage, 0.0);

        if self.json {
            let output = serde_json::json!({
                "safety": safety,
                "friction": friction,
                "coverage": {
                    "score": coverage.score,
                    "critical_coverage": coverage.critical_coverage,
                    "covered_categories": coverage.covered_categories,
                    "total_categories": coverage.total_categories,
                    "gaps": coverage.gaps,
                },
                "convergence": {
                    "converged": convergence.converged,
                    "all_rules_converged": convergence.all_rules_converged,
                    "coverage_satisfied": convergence.coverage_satisfied,
                    "stable": convergence.stable,
                },
                "rules": rule_health,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
            return Ok(());
        }

        println!("Governance Status\n");

        // Scores
        print_score("Safety Score (S)", safety, 0.80);
        print_score("Friction Score (F)", friction, 0.02);
        print_score("Coverage Score (C)", coverage.score, 0.50);
        println!(
            "  Categories: {}/{} covered (critical: {:.0}%)",
            coverage.covered_categories,
            coverage.total_categories,
            coverage.critical_coverage * 100.0
        );

        // Convergence
        let conv_status = if convergence.converged {
            "[ok] Converged"
        } else {
            "[..] Not converged"
        };
        println!("\n{}", conv_status);
        if !convergence.all_rules_converged {
            let unconverged: Vec<_> = convergence
                .rule_health
                .iter()
                .filter(|r| !r.converged)
                .map(|r| r.id.as_str())
                .collect();
            println!(
                "  Rules needing more observations: {}",
                unconverged.join(", ")
            );
        }

        // Coverage gaps
        if !coverage.gaps.is_empty() {
            println!("\nCoverage Gaps ({}):", coverage.gaps.len());
            for gap in &coverage.gaps {
                println!("  - {} ({} severity)", gap.category_id, gap.severity);
                if !gap.examples.is_empty() {
                    println!(
                        "    Examples: {}",
                        gap.examples[..gap.examples.len().min(3)].join(", ")
                    );
                }
            }
        }

        // Active rules with health
        println!("\nActive Rules ({}):", rules.len());
        for rh in &rule_health {
            let icon = if rh.converged {
                "+"
            } else if rh.precision >= 0.9 {
                "~"
            } else {
                "!"
            };
            println!(
                "  [{}] {} — precision={:.0}%, CI={:.3}, obs={}{}",
                icon,
                rh.id,
                rh.precision * 100.0,
                rh.confidence_interval,
                rh.observations,
                if rh.converged { " (converged)" } else { "" }
            );
        }

        // Recommendations
        let high_gaps: Vec<_> = coverage
            .gaps
            .iter()
            .filter(|g| g.severity == "critical" || g.severity == "high")
            .collect();
        let flagged: Vec<_> = rules
            .iter()
            .filter(|r| {
                let fp = r.beta as f64 / (r.alpha + r.beta) as f64;
                fp > 0.2 && r.beta > 2
            })
            .collect();

        if !high_gaps.is_empty() || !flagged.is_empty() {
            println!("\nRecommendations:");
            if !high_gaps.is_empty() {
                println!(
                    "  - Add rules for {} uncovered high/critical categories: {}",
                    high_gaps.len(),
                    high_gaps
                        .iter()
                        .map(|g| g.category_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            for rule in &flagged {
                let fp_rate = rule.beta as f64 / (rule.alpha + rule.beta) as f64 * 100.0;
                println!(
                    "  - Review '{}' (FP rate: {:.0}%, beta={})",
                    rule.id, fp_rate, rule.beta
                );
            }
        }

        Ok(())
    }
}

fn print_score(label: &str, value: f64, target: f64) {
    let status = if value >= target { "[ok]" } else { "[!!]" };
    println!(
        "{} {:<22} {:.3}  (target: >={:.2})",
        status, label, value, target
    );
}
