use crate::core::storage;
use anyhow::Result;
use clap::{Args, Subcommand};

use crate::cmd::{lifecycle, optimize, probe};

/// Manage governance rules.
#[derive(Args)]
pub struct RulesCmd {
    #[command(subcommand)]
    action: RulesAction,

    /// Database URL (defaults to ~/.synodic/synodic.db)
    #[arg(long, env = "DATABASE_URL", global = true)]
    db_url: Option<String>,
}

#[derive(Subcommand)]
enum RulesAction {
    /// List all rules
    List {
        /// Show all rules, including disabled/deprecated
        #[arg(long)]
        all: bool,
    },
    /// Show a single rule's details
    Show {
        /// Rule ID
        id: String,
    },

    // ── Lifecycle management ────────────────────────────────
    /// Promote a candidate rule to active
    Promote {
        /// Rule ID to promote
        id: String,
    },
    /// Crystallize a tuned rule to L1 git hook
    Crystallize {
        /// Rule ID to crystallize
        id: String,
    },
    /// Deprecate a rule (disable it)
    Deprecate {
        /// Rule ID to deprecate
        id: String,
    },
    /// Check all active rules for auto-transitions
    Check,

    // ── Adversarial probing ─────────────────────────────────
    /// Test rules against evasion variants
    Probe {
        /// Probe a specific rule (default: all active)
        #[arg(long)]
        rule: Option<String>,

        /// Use a specific strategy
        #[arg(long)]
        strategy: Option<String>,

        /// Auto-apply safe pattern expansions
        #[arg(long)]
        auto_apply: bool,
    },

    // ── Optimization ────────────────────────────────────────
    /// Scan feedback and propose rule candidates from patterns
    Optimize {
        /// Only show proposals, don't create candidates
        #[arg(long)]
        dry_run: bool,

        /// Lookback period in days
        #[arg(long, default_value = "90")]
        days: i64,
    },
}

impl RulesCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = self
            .db_url
            .unwrap_or_else(storage::pool::resolve_database_url);
        let store = storage::pool::create_storage(&db_url).await?;

        match self.action {
            RulesAction::List { all } => list_rules(&*store, !all).await,
            RulesAction::Show { id } => show_rule(&*store, &id).await,

            // Lifecycle
            RulesAction::Promote { id } => lifecycle::promote(&*store, &id).await,
            RulesAction::Crystallize { id } => lifecycle::crystallize(&*store, &id).await,
            RulesAction::Deprecate { id } => lifecycle::deprecate(&*store, &id).await,
            RulesAction::Check => lifecycle::check_transitions(&*store).await,

            // Probe
            RulesAction::Probe {
                rule,
                strategy,
                auto_apply,
            } => probe::run_probes(&*store, rule.as_deref(), strategy.as_deref(), auto_apply).await,

            // Optimize
            RulesAction::Optimize { dry_run, days } => {
                optimize::run_optimize(&*store, dry_run, days).await
            }
        }
    }
}

async fn list_rules(store: &dyn storage::Storage, active_only: bool) -> Result<()> {
    let rules = store.get_rules(active_only).await?;

    if rules.is_empty() {
        println!("No rules found.");
        return Ok(());
    }

    println!(
        "{:<25} {:<12} {:<20} {:<10} {:>5} {:>5} {:>7}",
        "ID", "LIFECYCLE", "CATEGORY", "ENABLED", "ALPHA", "BETA", "PREC %"
    );
    println!("{}", "-".repeat(90));

    for rule in &rules {
        let precision = if rule.alpha + rule.beta > 0 {
            (rule.alpha as f64 / (rule.alpha + rule.beta) as f64) * 100.0
        } else {
            0.0
        };

        println!(
            "{:<25} {:<12} {:<20} {:<10} {:>5} {:>5} {:>6.1}",
            rule.id,
            rule.lifecycle.as_str(),
            rule.category_id,
            if rule.enabled { "yes" } else { "no" },
            rule.alpha,
            rule.beta,
            precision,
        );
    }

    println!("\n{} rule(s)", rules.len());
    Ok(())
}

async fn show_rule(store: &dyn storage::Storage, id: &str) -> Result<()> {
    let rule = store
        .get_rule(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("rule '{}' not found", id))?;

    let precision = if rule.alpha + rule.beta > 0 {
        (rule.alpha as f64 / (rule.alpha + rule.beta) as f64) * 100.0
    } else {
        0.0
    };

    println!("Rule: {}", rule.id);
    println!("  Description:  {}", rule.description);
    println!("  Category:     {}", rule.category_id);
    println!("  Lifecycle:    {}", rule.lifecycle);
    println!("  Enabled:      {}", rule.enabled);
    println!(
        "  Tools:        {}",
        if rule.tools.is_empty() {
            "all".to_string()
        } else {
            rule.tools.join(", ")
        }
    );
    println!(
        "  Condition:    {} = {}",
        rule.condition_type, rule.condition_value
    );
    println!("  Alpha (TP):   {}", rule.alpha);
    println!("  Beta (FP):    {}", rule.beta);
    println!("  Precision:    {:.1}%", precision);
    println!("  Created:      {}", rule.created_at);
    println!("  Updated:      {}", rule.updated_at);

    if let Some(ts) = rule.crystallized_at {
        println!("  Crystallized: {}", ts);
    }

    // Show recent feedback
    let feedback = store
        .get_feedback(storage::FeedbackFilters {
            rule_id: Some(id.to_string()),
            limit: Some(5),
            ..Default::default()
        })
        .await?;

    if !feedback.is_empty() {
        println!("\n  Recent feedback:");
        for event in &feedback {
            let reason = event
                .override_reason
                .as_deref()
                .map(|r| format!(" — {r}"))
                .unwrap_or_default();
            println!(
                "    {} {} {}{}",
                event.created_at.format("%Y-%m-%d %H:%M"),
                event.signal_type,
                event.tool_name,
                reason
            );
        }
    }

    Ok(())
}
