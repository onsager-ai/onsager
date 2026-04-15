use crate::core::storage::{self, CreateRule, FeedbackFilters, Lifecycle};
use anyhow::Result;
use clap::Args;

/// Scan feedback events and propose rule candidates from recurring patterns.
///
/// Looks for CI failures and incidents that match uncovered threat categories,
/// and creates candidate rules when patterns appear ≥3 times.
#[derive(Args)]
pub struct OptimizeCmd {
    /// Only show proposals, don't create candidates
    #[arg(long)]
    dry_run: bool,

    /// Lookback period in days (default: 90)
    #[arg(long, default_value = "90")]
    days: i64,

    /// Database URL
    #[arg(long, env = "DATABASE_URL")]
    db_url: Option<String>,
}

impl OptimizeCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = self
            .db_url
            .unwrap_or_else(storage::pool::resolve_database_url);
        let store = storage::pool::create_storage(&db_url).await?;

        run_optimize(&*store, self.dry_run, self.days).await
    }
}

pub async fn run_optimize(store: &dyn storage::Storage, dry_run: bool, days: i64) -> Result<()> {
    let since = chrono::Utc::now() - chrono::Duration::days(days);

    println!("Scanning feedback events (last {} days)...\n", days);

    // 1. Check for rules that should be flagged/deprecated
    let rules = store.get_rules(false).await?;
    let mut flagged = 0;

    for rule in &rules {
        if rule.lifecycle == Lifecycle::Deprecated || rule.lifecycle == Lifecycle::Crystallized {
            continue;
        }

        let observations = rule.alpha + rule.beta;
        let fp_rate = if observations > 0 {
            rule.beta as f64 / observations as f64
        } else {
            0.0
        };

        if fp_rate > 0.4 && observations >= 10 {
            println!(
                "  FLAG: Rule '{}' has {:.0}% false positive rate (beta={})",
                rule.id,
                fp_rate * 100.0,
                rule.beta
            );
            println!("    Run: synodic rules deprecate {}", rule.id);
            flagged += 1;
        } else if fp_rate > 0.2 && observations >= 5 {
            println!(
                "  WARN: Rule '{}' has {:.0}% false positive rate — review recommended",
                rule.id,
                fp_rate * 100.0
            );
            flagged += 1;
        }
    }

    if flagged > 0 {
        println!();
    }

    // 2. Look at CI failures for patterns
    let ci_failures = store
        .get_feedback(FeedbackFilters {
            signal_type: Some("ci_failure".to_string()),
            since: Some(since),
            ..Default::default()
        })
        .await?;

    let incidents = store
        .get_feedback(FeedbackFilters {
            signal_type: Some("incident".to_string()),
            since: Some(since),
            ..Default::default()
        })
        .await?;

    // 3. Check coverage gaps
    let categories = store.get_threat_categories().await?;
    let active_rules = store.get_rules(true).await?;

    let uncovered: Vec<_> = categories
        .iter()
        .filter(|c| !active_rules.iter().any(|r| r.category_id == c.id))
        .collect();

    if !uncovered.is_empty() {
        println!("Coverage gaps ({}):", uncovered.len());
        for cat in &uncovered {
            let relevant_events = ci_failures
                .iter()
                .chain(incidents.iter())
                .filter(|e| {
                    let input_str = e.tool_input.to_string().to_lowercase();
                    cat.examples
                        .iter()
                        .any(|ex| input_str.contains(&ex.to_lowercase()))
                })
                .count();

            println!(
                "  - {} ({} severity) — {} related events",
                cat.id, cat.severity, relevant_events
            );

            if relevant_events >= 3 && !dry_run {
                let rule_id = format!("auto-{}", cat.id);
                let pattern = cat
                    .examples
                    .iter()
                    .map(|e| regex::escape(e))
                    .collect::<Vec<_>>()
                    .join("|");

                if store.get_rule(&rule_id).await?.is_none() {
                    store
                        .create_rule(CreateRule {
                            id: rule_id.clone(),
                            description: format!("Auto-generated candidate for {} threats", cat.id),
                            category_id: cat.id.clone(),
                            tools: vec!["Bash".to_string()],
                            condition_type: "command".to_string(),
                            condition_value: pattern,
                            lifecycle: Lifecycle::Candidate,
                            prior_alpha: 1,
                            prior_beta: 1,
                            project_id: None,
                        })
                        .await?;
                    println!(
                        "    -> Created candidate rule '{}' (run: synodic rules promote {})",
                        rule_id, rule_id
                    );
                }
            }
        }
    }

    // Summary
    println!("\nSummary:");
    println!(
        "  Active rules: {} / Flagged: {}",
        active_rules.len(),
        flagged
    );
    println!(
        "  Coverage: {}/{}",
        categories.len() - uncovered.len(),
        categories.len()
    );
    println!(
        "  CI failures ({}d): {} / Incidents: {}",
        days,
        ci_failures.len(),
        incidents.len()
    );

    if dry_run {
        println!("\n  (dry run — no candidates created)");
    }

    Ok(())
}
