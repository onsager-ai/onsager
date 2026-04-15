use crate::core::intercept::{self, InterceptEngine, InterceptRequest};
use anyhow::Result;
use clap::Args;

/// Evaluate an agent tool call against interception rules.
///
/// Designed for use as a Claude Code PreToolUse hook:
///
///   hooks:
///     - matcher: "PreToolUse"
///       command: "synodic intercept --tool $CLAUDE_TOOL_NAME --input $CLAUDE_TOOL_INPUT"
///
/// Returns JSON: {"decision": "allow"} or {"decision": "block", "reason": "..."}
#[derive(Args)]
pub struct InterceptCmd {
    /// Tool name (e.g., Write, Bash, Edit, Read)
    #[arg(long)]
    tool: String,

    /// Tool input as JSON string
    #[arg(long)]
    input: String,

    /// Path to custom rules file (YAML). Uses default rules if omitted.
    #[arg(long)]
    rules: Option<String>,
}

impl InterceptCmd {
    pub fn run(self) -> Result<()> {
        let tool_input: serde_json::Value = serde_json::from_str(&self.input)
            .unwrap_or_else(|_| serde_json::json!({ "raw": self.input }));

        let request = InterceptRequest {
            tool_name: self.tool,
            tool_input,
        };

        let rules = match self.rules {
            Some(path) => load_rules_file(&path)?,
            None => intercept::default_rules(),
        };

        let engine = InterceptEngine::new(rules);
        let response = engine.evaluate(&request);

        println!("{}", serde_json::to_string(&response)?);
        Ok(())
    }
}

fn load_rules_file(path: &str) -> Result<Vec<intercept::InterceptRule>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading rules file {path}: {e}"))?;
    let rules: Vec<intercept::InterceptRule> = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("parsing rules file {path}: {e}"))?;
    Ok(rules)
}
