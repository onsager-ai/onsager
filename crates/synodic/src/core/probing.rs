//! Adversarial probing — generate evasion variants and test rule robustness.
//!
//! Five strategies:
//! 1. Syntactic variation — flag forms, quoting, whitespace
//! 2. Indirection — subshells, eval, aliases
//! 3. Encoding — base64, variable interpolation
//! 4. Semantic equivalence — alternative tools for same effect
//! 5. Path traversal — relative paths, symlinks, canonicalization

use serde::{Deserialize, Serialize};

use crate::core::intercept::{
    InterceptCondition, InterceptEngine, InterceptRequest, InterceptRule,
};
use crate::core::storage::Rule;

/// Result of probing a rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReport {
    pub rule_id: String,
    pub strategy: String,
    pub variants: Vec<ProbeVariant>,
}

/// A single probe variant and whether it bypassed the rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeVariant {
    pub input: String,
    pub bypassed: bool,
}

/// A proposed rule expansion to fix a bypass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionProposal {
    pub rule_id: String,
    pub bypass_variant: String,
    pub proposed_pattern: String,
    pub safe_commands_blocked: Vec<String>,
    pub safe_to_apply: bool,
}

/// Strategy trait for generating adversarial variants.
pub trait ProbeStrategy {
    fn name(&self) -> &'static str;
    fn applicable(&self, rule: &Rule) -> bool;
    fn generate(&self, rule: &Rule) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Strategy 1: Syntactic variation
// ---------------------------------------------------------------------------

pub struct SyntacticVariation;

impl ProbeStrategy for SyntacticVariation {
    fn name(&self) -> &'static str {
        "syntactic-variation"
    }

    fn applicable(&self, rule: &Rule) -> bool {
        rule.condition_type == "command"
    }

    fn generate(&self, rule: &Rule) -> Vec<String> {
        let mut variants = Vec::new();
        let pattern = &rule.condition_value;

        // Detect and generate flag variants
        if pattern.contains("--force") {
            variants.push(
                rule.condition_value
                    .replace("--force", "-f")
                    .replace("\\s+", " "),
            );
            // Try the literal command that matches
            if pattern.contains("push") {
                variants.push("git push -f origin main".to_string());
                variants.push("git push --force-with-lease origin main".to_string());
            }
        }
        if pattern.contains("--hard") {
            variants.push("git reset --hard~1".to_string());
        }

        // Double space variant
        if pattern.contains("push") {
            variants.push("git  push --force origin main".to_string());
        }

        // rm variants
        if pattern.contains("rm") {
            variants.push("rm -Rf /".to_string());
            variants.push("rm -r -f /".to_string());
            variants.push("rm --recursive --force /".to_string());
        }

        variants
    }
}

// ---------------------------------------------------------------------------
// Strategy 2: Indirection
// ---------------------------------------------------------------------------

pub struct Indirection;

impl ProbeStrategy for Indirection {
    fn name(&self) -> &'static str {
        "indirection"
    }

    fn applicable(&self, rule: &Rule) -> bool {
        rule.condition_type == "command"
    }

    fn generate(&self, rule: &Rule) -> Vec<String> {
        // Extract a representative command from the pattern
        let commands = extract_representative_commands(rule);

        let mut variants = Vec::new();
        for cmd in &commands {
            variants.push(format!("bash -c '{cmd}'"));
            variants.push(format!("sh -c '{cmd}'"));
            variants.push(format!("eval \"{cmd}\""));
            variants.push(format!("echo '{cmd}' | bash"));
        }
        variants
    }
}

// ---------------------------------------------------------------------------
// Strategy 3: Encoding
// ---------------------------------------------------------------------------

pub struct Encoding;

impl ProbeStrategy for Encoding {
    fn name(&self) -> &'static str {
        "encoding"
    }

    fn applicable(&self, rule: &Rule) -> bool {
        rule.condition_type == "command"
    }

    fn generate(&self, rule: &Rule) -> Vec<String> {
        let commands = extract_representative_commands(rule);

        let mut variants = Vec::new();
        for cmd in &commands {
            // Variable interpolation
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.len() >= 2 {
                let var_form = parts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| format!("v{i}={p}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                let var_refs = (0..parts.len())
                    .map(|i| format!("$v{i}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                variants.push(format!("{var_form}; {var_refs}"));
            }
        }
        variants
    }
}

// ---------------------------------------------------------------------------
// Strategy 4: Semantic equivalence
// ---------------------------------------------------------------------------

pub struct SemanticEquivalence;

impl ProbeStrategy for SemanticEquivalence {
    fn name(&self) -> &'static str {
        "semantic-equivalence"
    }

    fn applicable(&self, rule: &Rule) -> bool {
        rule.condition_type == "command"
    }

    fn generate(&self, rule: &Rule) -> Vec<String> {
        let pattern = &rule.condition_value;
        let mut variants = Vec::new();

        if pattern.contains("rm") {
            variants.push("unlink /important-file".to_string());
            variants.push("find / -delete".to_string());
            variants.push("python3 -c 'import shutil; shutil.rmtree(\"/\")'".to_string());
            variants.push("perl -e 'use File::Path; rmtree(\"/\")'".to_string());
        }

        if pattern.contains("push") && pattern.contains("force") {
            variants.push("git push origin +main".to_string()); // + prefix = force
        }

        if pattern.contains("reset") && pattern.contains("hard") {
            variants.push("git checkout -- .".to_string());
            variants.push("git restore .".to_string());
        }

        variants
    }
}

// ---------------------------------------------------------------------------
// Strategy 5: Path traversal
// ---------------------------------------------------------------------------

pub struct PathTraversal;

impl ProbeStrategy for PathTraversal {
    fn name(&self) -> &'static str {
        "path-traversal"
    }

    fn applicable(&self, rule: &Rule) -> bool {
        rule.condition_type == "path"
    }

    fn generate(&self, rule: &Rule) -> Vec<String> {
        let glob = &rule.condition_value;
        let mut variants = Vec::new();

        if glob.starts_with("/etc") {
            variants.push("/etc/../etc/passwd".to_string());
            variants.push("../../../../../../etc/passwd".to_string());
            variants.push("/etc/./passwd".to_string());
        }

        if glob.starts_with("/usr") {
            variants.push("/usr/../usr/local/bin/test".to_string());
            variants.push("../../../../../../usr/local/bin/test".to_string());
        }

        variants
    }
}

// ---------------------------------------------------------------------------
// Probe runner
// ---------------------------------------------------------------------------

/// Run a probe strategy against a rule.
pub fn run_probe(rule: &Rule, strategy: &dyn ProbeStrategy) -> ProbeReport {
    let variants_input = strategy.generate(rule);
    let intercept_rule: InterceptRule = rule.into();
    let engine = InterceptEngine::new(vec![intercept_rule]);

    let mut variants = Vec::new();

    for input in variants_input {
        let tool = if rule.condition_type == "path" {
            "Write"
        } else {
            "Bash"
        };

        let request = if rule.condition_type == "path" {
            InterceptRequest {
                tool_name: tool.to_string(),
                tool_input: serde_json::json!({ "file_path": input }),
            }
        } else {
            InterceptRequest {
                tool_name: tool.to_string(),
                tool_input: serde_json::json!({ "command": input }),
            }
        };

        let response = engine.evaluate(&request);
        let bypassed = response.decision == "allow";

        variants.push(ProbeVariant { input, bypassed });
    }

    ProbeReport {
        rule_id: rule.id.clone(),
        strategy: strategy.name().to_string(),
        variants,
    }
}

/// Run all applicable strategies against a rule.
pub fn run_all_probes(rule: &Rule) -> Vec<ProbeReport> {
    let strategies: Vec<Box<dyn ProbeStrategy>> = vec![
        Box::new(SyntacticVariation),
        Box::new(Indirection),
        Box::new(Encoding),
        Box::new(SemanticEquivalence),
        Box::new(PathTraversal),
    ];

    strategies
        .iter()
        .filter(|s| s.applicable(rule))
        .map(|s| run_probe(rule, s.as_ref()))
        .collect()
}

/// Backtest a proposed pattern expansion against known-safe commands.
pub fn backtest_expansion(proposed_pattern: &str, condition_type: &str) -> ExpansionProposal {
    let safe_commands = if condition_type == "path" {
        vec![
            "/home/user/project/src/main.rs",
            "/home/user/project/Cargo.toml",
            "/tmp/test.txt",
        ]
    } else {
        vec![
            "git status",
            "git log --oneline",
            "git diff",
            "git checkout main",
            "git push -u origin feature",
            "cargo build",
            "cargo test",
            "npm install",
            "ls -la",
            "rm -rf target/debug",
        ]
    };

    let condition = if condition_type == "path" {
        InterceptCondition::Path {
            glob: proposed_pattern.to_string(),
        }
    } else {
        InterceptCondition::Command {
            pattern: proposed_pattern.to_string(),
        }
    };

    let rule = InterceptRule {
        id: "backtest".to_string(),
        description: "backtest".to_string(),
        tools: vec![],
        condition,
    };

    let engine = InterceptEngine::new(vec![rule]);
    let mut blocked = Vec::new();

    for cmd in &safe_commands {
        let request = if condition_type == "path" {
            InterceptRequest {
                tool_name: "Write".to_string(),
                tool_input: serde_json::json!({ "file_path": cmd }),
            }
        } else {
            InterceptRequest {
                tool_name: "Bash".to_string(),
                tool_input: serde_json::json!({ "command": cmd }),
            }
        };

        if engine.evaluate(&request).decision == "block" {
            blocked.push(cmd.to_string());
        }
    }

    ExpansionProposal {
        rule_id: String::new(),
        bypass_variant: String::new(),
        proposed_pattern: proposed_pattern.to_string(),
        safe_to_apply: blocked.is_empty(),
        safe_commands_blocked: blocked,
    }
}

/// Generate an expanded pattern that catches both the original and a bypass variant.
pub fn expand_pattern(original: &str, variant: &str) -> String {
    // Simple approach: create alternation
    // A more sophisticated version would analyze the regex AST
    format!("({}|{})", original, regex::escape(variant))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract representative commands from a rule's regex pattern.
fn extract_representative_commands(rule: &Rule) -> Vec<String> {
    let pattern = &rule.condition_value;
    let mut commands = Vec::new();

    // Heuristic: detect common patterns and generate concrete examples
    if pattern.contains("push") && pattern.contains("force") {
        commands.push("git push --force origin main".to_string());
    }
    if pattern.contains("reset") && pattern.contains("hard") {
        commands.push("git reset --hard HEAD~1".to_string());
    }
    if pattern.contains("clean") {
        commands.push("git clean -fdx".to_string());
    }
    if pattern.contains("rm") {
        commands.push("rm -rf /".to_string());
    }

    if commands.is_empty() {
        // Fallback: use pattern as-is (may not be a valid command)
        commands.push(pattern.clone());
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::storage::Lifecycle;

    fn make_command_rule(id: &str, pattern: &str) -> Rule {
        Rule {
            id: id.to_string(),
            description: "test".to_string(),
            category_id: "test".to_string(),
            tools: vec!["Bash".to_string()],
            condition_type: "command".to_string(),
            condition_value: pattern.to_string(),
            lifecycle: Lifecycle::Active,
            alpha: 1,
            beta: 1,
            prior_alpha: 1,
            prior_beta: 1,
            enabled: true,
            project_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            crystallized_at: None,
            cross_project_validated: false,
        }
    }

    fn make_path_rule(id: &str, glob: &str) -> Rule {
        Rule {
            condition_type: "path".to_string(),
            condition_value: glob.to_string(),
            tools: vec!["Write".to_string(), "Edit".to_string()],
            ..make_command_rule(id, glob)
        }
    }

    #[test]
    fn syntactic_variation_generates_variants() {
        let rule = make_command_rule(
            "destructive-git",
            r"git\s+(reset\s+--hard|push\s+--force|push\s+-f|clean\s+-fd)\b",
        );
        let strategy = SyntacticVariation;
        let variants = strategy.generate(&rule);
        assert!(!variants.is_empty(), "should generate variants");
    }

    #[test]
    fn indirection_generates_subshell_variants() {
        let rule = make_command_rule("destructive-git", r"git\s+(push\s+--force)\b");
        let strategy = Indirection;
        let variants = strategy.generate(&rule);
        assert!(variants.iter().any(|v| v.contains("bash -c")));
        assert!(variants.iter().any(|v| v.contains("sh -c")));
    }

    #[test]
    fn probe_finds_bypasses() {
        let rule = make_command_rule(
            "destructive-git",
            r"git\s+(reset\s+--hard|push\s+--force|push\s+-f|clean\s+-fd)\b",
        );

        let reports = run_all_probes(&rule);
        assert!(!reports.is_empty());

        // Indirection should find bypasses (bash -c wrapping)
        let indirection = reports.iter().find(|r| r.strategy == "indirection");
        assert!(indirection.is_some());
        let bypasses: Vec<_> = indirection
            .unwrap()
            .variants
            .iter()
            .filter(|v| v.bypassed)
            .collect();
        assert!(
            !bypasses.is_empty(),
            "indirection should bypass regex-based rules"
        );
    }

    #[test]
    fn semantic_equivalence_finds_alternatives() {
        let rule = make_command_rule("dangerous-rm", r"rm\s+-[rR]f?\s+(/\s|/$)");
        let strategy = SemanticEquivalence;
        let variants = strategy.generate(&rule);
        assert!(variants.iter().any(|v| v.contains("unlink")));
        assert!(variants.iter().any(|v| v.contains("find")));
    }

    #[test]
    fn path_traversal_generates_variants() {
        let rule = make_path_rule("writes-outside-project", "/etc/**");
        let strategy = PathTraversal;
        assert!(strategy.applicable(&rule));
        let variants = strategy.generate(&rule);
        assert!(variants.iter().any(|v| v.contains("..")));
    }

    #[test]
    fn backtest_safe_pattern_passes() {
        let result = backtest_expansion(r"git\s+push\s+--force", "command");
        assert!(result.safe_to_apply);
        assert!(result.safe_commands_blocked.is_empty());
    }

    #[test]
    fn backtest_overbroad_pattern_fails() {
        let result = backtest_expansion(r"git", "command");
        assert!(!result.safe_to_apply);
        assert!(!result.safe_commands_blocked.is_empty());
    }

    #[test]
    fn expand_pattern_creates_alternation() {
        let expanded = expand_pattern(r"git\s+push\s+--force", "git push -f");
        assert!(expanded.contains('|'));
        assert!(expanded.contains("git push \\-f"));
    }

    #[test]
    fn strategies_not_applicable_to_wrong_type() {
        let path_rule = make_path_rule("test", "/etc/**");
        assert!(!SyntacticVariation.applicable(&path_rule));
        assert!(!Indirection.applicable(&path_rule));
        assert!(PathTraversal.applicable(&path_rule));

        let cmd_rule = make_command_rule("test", "rm -rf");
        assert!(SyntacticVariation.applicable(&cmd_rule));
        assert!(!PathTraversal.applicable(&cmd_rule));
    }
}
