use crate::core::{
    scoring::{self, beta_confidence_interval, validate_constitutional},
    storage::{self, Lifecycle, UpdateRule},
};
use anyhow::Result;
use clap::{Args, Subcommand};

/// Manage rule lifecycle transitions (promote, crystallize, deprecate).
#[derive(Args)]
pub struct LifecycleCmd {
    #[command(subcommand)]
    action: LifecycleAction,

    /// Database URL
    #[arg(long, env = "DATABASE_URL", global = true)]
    db_url: Option<String>,
}

#[derive(Subcommand)]
enum LifecycleAction {
    /// Promote a candidate rule to active (requires clear and convincing evidence)
    Promote {
        /// Rule ID to promote
        id: String,
    },
    /// Crystallize a tuned rule to L1 git hook (requires beyond reasonable doubt)
    Crystallize {
        /// Rule ID to crystallize
        id: String,
    },
    /// Deprecate a rule (disable due to high false positive rate)
    Deprecate {
        /// Rule ID to deprecate
        id: String,
    },
    /// Check all active rules for auto-transitions (tuned, deprecated)
    Check,
}

impl LifecycleCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = self
            .db_url
            .unwrap_or_else(storage::pool::resolve_database_url);
        let store = storage::pool::create_storage(&db_url).await?;

        match self.action {
            LifecycleAction::Promote { id } => promote(&*store, &id).await,
            LifecycleAction::Crystallize { id } => crystallize(&*store, &id).await,
            LifecycleAction::Deprecate { id } => deprecate(&*store, &id).await,
            LifecycleAction::Check => check_transitions(&*store).await,
        }
    }
}

/// Promote candidate → active.
///
/// Evidentiary standard: **Clear and convincing**
/// - Backtest precision > 0.9
/// - ≥5 observations
/// - Passes constitutional constraints
pub async fn promote(store: &dyn storage::Storage, id: &str) -> Result<()> {
    let rule = store
        .get_rule(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("rule '{}' not found", id))?;

    if rule.lifecycle != Lifecycle::Candidate {
        anyhow::bail!(
            "rule '{}' is {:?}, not candidate — cannot promote",
            id,
            rule.lifecycle
        );
    }

    // Check observations
    let observations = rule.alpha + rule.beta;
    if observations < 5 {
        anyhow::bail!(
            "insufficient evidence: {} observations (need >=5)",
            observations
        );
    }

    // Check precision
    let precision = rule.alpha as f64 / observations as f64;
    if precision < 0.9 {
        anyhow::bail!(
            "backtest precision {:.1}% < 90% — rule has too many false positives",
            precision * 100.0
        );
    }

    // Constitutional constraints
    if let Err(violations) =
        validate_constitutional(&rule.tools, &rule.condition_type, &rule.description)
    {
        for v in &violations {
            eprintln!("Constitutional violation: {}", v);
        }
        anyhow::bail!(
            "rule violates {} constitutional constraint(s)",
            violations.len()
        );
    }

    store
        .update_rule(
            id,
            UpdateRule {
                lifecycle: Some(Lifecycle::Active),
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await?;

    println!(
        "Promoted rule '{}' to active (precision: {:.1}%, observations: {})",
        id,
        precision * 100.0,
        observations
    );

    Ok(())
}

/// Crystallize tuned → L1 git hook.
///
/// Evidentiary standard: **Beyond reasonable doubt**
/// - alpha > 30
/// - precision > 0.95
/// - CI < 0.1
/// - Cross-project validated
/// - 30+ days as tuned
pub async fn crystallize(store: &dyn storage::Storage, id: &str) -> Result<()> {
    let rule = store
        .get_rule(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("rule '{}' not found", id))?;

    if rule.lifecycle != Lifecycle::Tuned {
        anyhow::bail!(
            "rule '{}' is {:?}, not tuned — cannot crystallize",
            id,
            rule.lifecycle
        );
    }

    // Check alpha threshold
    if rule.alpha < 30 {
        anyhow::bail!(
            "insufficient true positives: alpha={} (need >=30)",
            rule.alpha
        );
    }

    // Check precision
    let precision = rule.alpha as f64 / (rule.alpha + rule.beta) as f64;
    if precision < 0.95 {
        anyhow::bail!(
            "precision {:.1}% < 95% — not ready for crystallization",
            precision * 100.0
        );
    }

    // Check CI
    let ci = beta_confidence_interval(rule.alpha, rule.beta);
    if ci >= 0.1 {
        anyhow::bail!("confidence interval {:.3} >= 0.1 — not converged", ci);
    }

    // Check cross-project validation
    if !rule.cross_project_validated {
        anyhow::bail!("rule not validated across >=2 projects — set cross_project_validated first");
    }

    // Check age (30+ days)
    let age_days = (chrono::Utc::now() - rule.created_at).num_days();
    if age_days < 30 {
        anyhow::bail!(
            "rule only {} days old (need >=30 for crystallization)",
            age_days
        );
    }

    // Generate git hook check
    let hook_check = generate_git_hook_check(&rule)?;

    println!("Crystallizing rule '{}' to L1 git hook\n", id);
    println!("Generated hook check:");
    println!("{}", hook_check);
    println!();
    println!("Add this to .githooks/pre-commit to enforce at L1.");
    println!("Note: L2 rule remains active as backup.\n");

    store
        .update_rule(
            id,
            UpdateRule {
                lifecycle: Some(Lifecycle::Crystallized),
                crystallized_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
        )
        .await?;

    println!(
        "Rule '{}' crystallized (alpha={}, precision={:.1}%)",
        id,
        rule.alpha,
        precision * 100.0
    );

    Ok(())
}

/// Deprecate a rule.
pub async fn deprecate(store: &dyn storage::Storage, id: &str) -> Result<()> {
    let rule = store
        .get_rule(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("rule '{}' not found", id))?;

    if rule.lifecycle == Lifecycle::Deprecated {
        println!("Rule '{}' is already deprecated", id);
        return Ok(());
    }

    let fp_rate = if rule.alpha + rule.beta > 0 {
        rule.beta as f64 / (rule.alpha + rule.beta) as f64
    } else {
        0.0
    };

    store
        .update_rule(
            id,
            UpdateRule {
                lifecycle: Some(Lifecycle::Deprecated),
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await?;

    println!(
        "Deprecated rule '{}' (FP rate: {:.1}%, alpha={}, beta={})",
        id,
        fp_rate * 100.0,
        rule.alpha,
        rule.beta
    );

    // Check coverage impact
    let categories = store.get_threat_categories().await?;
    let remaining_rules = store.get_rules(true).await?;
    let coverage = scoring::compute_coverage(&remaining_rules, &categories);

    if coverage.score < 0.5 {
        eprintln!(
            "WARNING: Coverage dropped to {:.1}% after deprecation",
            coverage.score * 100.0
        );
    }

    Ok(())
}

/// Check all active rules for automatic lifecycle transitions.
pub async fn check_transitions(store: &dyn storage::Storage) -> Result<()> {
    let rules = store.get_rules(false).await?;
    let mut transitions = 0;

    for rule in &rules {
        if rule.lifecycle != Lifecycle::Active {
            continue;
        }

        let observations = rule.alpha + rule.beta;
        let ci = beta_confidence_interval(rule.alpha, rule.beta);
        let fp_rate = if observations > 0 {
            rule.beta as f64 / observations as f64
        } else {
            0.0
        };
        let age_days = (chrono::Utc::now() - rule.created_at).num_days();

        // Active → Deprecated (high FP rate)
        if fp_rate > 0.4 && observations >= 10 {
            store
                .update_rule(
                    &rule.id,
                    UpdateRule {
                        lifecycle: Some(Lifecycle::Deprecated),
                        enabled: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
            println!(
                "  DEPRECATED '{}' — FP rate {:.0}% exceeds 40%",
                rule.id,
                fp_rate * 100.0
            );
            transitions += 1;
            continue;
        }

        // Active → Tuned (converged)
        if ci < 0.1 && observations > 20 && age_days >= 7 {
            store
                .update_rule(
                    &rule.id,
                    UpdateRule {
                        lifecycle: Some(Lifecycle::Tuned),
                        ..Default::default()
                    },
                )
                .await?;
            println!(
                "  TUNED '{}' — CI={:.3}, observations={}, age={}d",
                rule.id, ci, observations, age_days
            );
            transitions += 1;
            continue;
        }
    }

    if transitions == 0 {
        println!("No lifecycle transitions needed.");
    } else {
        println!("\n{} transition(s) applied.", transitions);
    }

    // Check system convergence
    let active_rules = store.get_rules(true).await?;
    let categories = store.get_threat_categories().await?;
    let coverage = scoring::compute_coverage(&active_rules, &categories);
    let convergence = scoring::check_convergence(&active_rules, &coverage, 0.0);

    if convergence.converged {
        println!("\nSystem has CONVERGED.");
    }

    Ok(())
}

/// Generate a git hook check from a rule (for crystallization).
fn generate_git_hook_check(rule: &storage::Rule) -> Result<String> {
    match rule.condition_type.as_str() {
        "command" => Ok(format!(
            r#"# Crystallized rule '{}': {}
if git diff --cached -p | grep -qE '{}'; then
  echo "Blocked by crystallized rule '{}': {}" >&2
  exit 1
fi"#,
            rule.id, rule.description, rule.condition_value, rule.id, rule.description
        )),
        "path" => {
            let regex_pattern = glob_to_regex(&rule.condition_value);
            Ok(format!(
                r#"# Crystallized rule '{}': {}
if git diff --cached --name-only | grep -qE '{}'; then
  echo "Blocked by crystallized rule '{}': {}" >&2
  exit 1
fi"#,
                rule.id, rule.description, regex_pattern, rule.id, rule.description
            ))
        }
        "pattern" => Ok(format!(
            r#"# Crystallized rule '{}': {}
if git diff --cached -p | grep -qE '{}'; then
  echo "Blocked by crystallized rule '{}': {}" >&2
  exit 1
fi"#,
            rule.id, rule.description, rule.condition_value, rule.id, rule.description
        )),
        other => anyhow::bail!(
            "condition type '{}' cannot be crystallized to git hook",
            other
        ),
    }
}

fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::new();
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        chars.next();
                    }
                    regex.push_str(".*");
                } else {
                    regex.push_str("[^/]*");
                }
            }
            '.' => regex.push_str("\\."),
            '?' => regex.push('.'),
            c if "()[]{}+^$|\\".contains(c) => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }
    regex
}
