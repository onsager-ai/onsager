use crate::core::storage::{self, FeedbackEvent, FeedbackFilters, UpdateRule};
use anyhow::Result;
use clap::{Args, Subcommand};
use uuid::Uuid;

/// Record governance feedback or analyze override reasons.
#[derive(Args)]
pub struct FeedbackCmd {
    #[command(subcommand)]
    action: Option<FeedbackAction>,

    // Flat args for the common case: `synodic feedback --rule X --signal Y`
    /// Rule ID that triggered the event
    #[arg(long, global = true)]
    rule: Option<String>,

    /// Signal type: override, confirmed, ci_failure, incident
    #[arg(long, global = true)]
    signal: Option<String>,

    /// Override reason (free text, optional)
    #[arg(long, global = true)]
    reason: Option<String>,

    /// Link to CI run, incident report, etc.
    #[arg(long, global = true)]
    evidence: Option<String>,

    /// Session ID (auto-detected from CLAUDE_SESSION_ID if available)
    #[arg(long, global = true)]
    session: Option<String>,

    /// Tool that was called
    #[arg(long, global = true)]
    tool: Option<String>,

    /// Tool input as JSON
    #[arg(long, global = true)]
    input: Option<String>,

    /// Database URL (defaults to ~/.synodic/synodic.db)
    #[arg(long, env = "DATABASE_URL", global = true)]
    db_url: Option<String>,
}

#[derive(Subcommand)]
enum FeedbackAction {
    /// Analyze override reasons for a rule (show clusters)
    Analyze {
        /// Rule ID to analyze
        rule_id: String,
    },
}

impl FeedbackCmd {
    pub async fn run(self) -> Result<()> {
        let db_url = self
            .db_url
            .clone()
            .unwrap_or_else(storage::pool::resolve_database_url);

        match self.action {
            Some(FeedbackAction::Analyze { rule_id }) => {
                let store = storage::pool::create_storage(&db_url).await?;
                analyze_overrides(&*store, &rule_id).await
            }
            None => {
                // Record feedback (flat args mode)
                let rule = self
                    .rule
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("--rule is required"))?;
                let signal = self
                    .signal
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("--signal is required"))?;
                record_feedback(&db_url, &rule, &signal, &self).await
            }
        }
    }
}

async fn record_feedback(db_url: &str, rule: &str, signal: &str, cmd: &FeedbackCmd) -> Result<()> {
    match signal {
        "override" | "confirmed" | "ci_failure" | "incident" => {}
        other => anyhow::bail!(
            "unknown signal type: {other} (expected: override, confirmed, ci_failure, incident)"
        ),
    }

    let store = storage::pool::create_storage(db_url).await?;

    if store.get_rule(rule).await?.is_none() {
        anyhow::bail!("rule '{}' not found", rule);
    }

    let session_id = cmd
        .session
        .clone()
        .or_else(|| std::env::var("CLAUDE_SESSION_ID").ok());

    let tool_input: serde_json::Value = cmd
        .input
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!({}));

    let event = FeedbackEvent {
        id: Uuid::new_v4(),
        signal_type: signal.to_string(),
        rule_id: rule.to_string(),
        session_id,
        tool_name: cmd.tool.clone().unwrap_or_else(|| "unknown".to_string()),
        tool_input,
        override_reason: cmd.reason.clone(),
        failure_type: None,
        evidence_url: cmd.evidence.clone(),
        project_id: None,
        created_at: chrono::Utc::now(),
    };

    store.record_feedback(event).await?;

    match signal {
        "override" => {
            store
                .update_rule(
                    rule,
                    UpdateRule {
                        beta_increment: Some(1),
                        ..Default::default()
                    },
                )
                .await?;
            let reason_note = cmd
                .reason
                .as_deref()
                .map(|r| format!(" (reason: {r})"))
                .unwrap_or_default();
            eprintln!("Recorded override for rule '{rule}'{reason_note}");
        }
        "confirmed" => {
            store
                .update_rule(
                    rule,
                    UpdateRule {
                        alpha_increment: Some(1),
                        ..Default::default()
                    },
                )
                .await?;
            eprintln!("Recorded confirmed block for rule '{rule}'");
        }
        _ => {
            eprintln!("Recorded {signal} for rule '{rule}'");
        }
    }

    Ok(())
}

async fn analyze_overrides(store: &dyn storage::Storage, rule_id: &str) -> Result<()> {
    use crate::core::clustering;

    let rule = store
        .get_rule(rule_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("rule '{}' not found", rule_id))?;

    let overrides = store
        .get_feedback(FeedbackFilters {
            rule_id: Some(rule_id.to_string()),
            signal_type: Some("override".to_string()),
            ..Default::default()
        })
        .await?;

    if overrides.is_empty() {
        println!("No overrides recorded for rule '{}'", rule_id);
        return Ok(());
    }

    let reasons: Vec<String> = overrides
        .iter()
        .filter_map(|e| e.override_reason.clone())
        .collect();

    let without_reason = overrides.len() - reasons.len();

    println!(
        "Override Analysis: '{}' ({} overrides, {} with reasons)\n",
        rule_id,
        overrides.len(),
        reasons.len()
    );

    // Rule stats
    let precision = rule.alpha as f64 / (rule.alpha + rule.beta) as f64 * 100.0;
    println!(
        "  Rule precision: {:.1}% (alpha={}, beta={})",
        precision, rule.alpha, rule.beta
    );

    let fp_rate = rule.beta as f64 / (rule.alpha + rule.beta) as f64;
    if fp_rate > 0.4 {
        println!(
            "  WARNING: False positive rate {:.0}% exceeds 40% threshold",
            fp_rate * 100.0
        );
    }
    println!();

    if reasons.is_empty() {
        println!("  No reasons provided with overrides.");
        if without_reason > 0 {
            println!("  {} overrides had no reason attached.", without_reason);
        }
        return Ok(());
    }

    let clusters = clustering::cluster_reasons(&reasons);

    for cluster in &clusters {
        println!(
            "  [{}] {} ({} occurrences)",
            cluster.cluster_id,
            cluster.label,
            cluster.reasons.len()
        );
        println!("    Suggestion: {}", cluster.suggestion);
        println!("    Examples:");
        for reason in cluster.reasons.iter().take(3) {
            println!("      - \"{}\"", reason);
        }
        if cluster.reasons.len() > 3 {
            println!("      ... and {} more", cluster.reasons.len() - 3);
        }
        println!();
    }

    if without_reason > 0 {
        println!(
            "  ({} overrides had no reason — consider encouraging reason capture)",
            without_reason
        );
    }

    Ok(())
}
