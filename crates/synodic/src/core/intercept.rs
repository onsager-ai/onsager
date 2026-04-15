//! L2 interception — evaluate agent tool calls before execution.
//!
//! This module implements the "L2 Intercept" quadrant from spec 069's
//! capability matrix. It evaluates agent tool calls against a set of
//! rules and returns allow/block decisions.
//!
//! Designed to be called from Claude Code `PreToolUse` hooks via the
//! `synodic intercept` CLI command. Must be fast (<100ms) — pure
//! pattern matching, no AI calls.

use serde::{Deserialize, Serialize};

/// A tool-agnostic representation of an agent's pending action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptRequest {
    /// What the agent is trying to do (e.g., "Write", "Bash", "Edit")
    pub tool_name: String,
    /// The arguments to the tool (e.g., file path, command string)
    pub tool_input: serde_json::Value,
}

/// The result of evaluating an intercept request against rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptResponse {
    /// "allow" or "block"
    pub decision: String,
    /// Reason for blocking (empty if allowed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Which rule triggered the block (empty if allowed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

impl InterceptResponse {
    pub fn allow() -> Self {
        Self {
            decision: "allow".to_string(),
            reason: None,
            rule: None,
        }
    }

    pub fn block(reason: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            decision: "block".to_string(),
            reason: Some(reason.into()),
            rule: Some(rule.into()),
        }
    }
}

/// A rule that can block agent tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptRule {
    /// Unique identifier for the rule
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Which tools this rule applies to (empty = all tools)
    #[serde(default)]
    pub tools: Vec<String>,
    /// The condition that triggers a block
    pub condition: InterceptCondition,
}

/// Conditions that can trigger a block decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InterceptCondition {
    /// Block if a regex matches the tool input (serialized as JSON string)
    #[serde(rename = "pattern")]
    Pattern { pattern: String },
    /// Block if the tool input contains a specific file path pattern
    #[serde(rename = "path")]
    Path { glob: String },
    /// Block commands matching a pattern (for Bash/shell tools)
    #[serde(rename = "command")]
    Command { pattern: String },
}

/// Engine that evaluates intercept requests against a set of rules.
pub struct InterceptEngine {
    rules: Vec<InterceptRule>,
}

impl InterceptEngine {
    pub fn new(rules: Vec<InterceptRule>) -> Self {
        Self { rules }
    }

    /// Evaluate a request against all rules. Returns the first block match,
    /// or allow if no rules match.
    pub fn evaluate(&self, request: &InterceptRequest) -> InterceptResponse {
        let input_str = request.tool_input.to_string();

        for rule in &self.rules {
            // Skip rules that don't apply to this tool
            if !rule.tools.is_empty()
                && !rule
                    .tools
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&request.tool_name))
            {
                continue;
            }

            if self.matches_condition(&rule.condition, request, &input_str) {
                return InterceptResponse::block(&rule.description, &rule.id);
            }
        }

        InterceptResponse::allow()
    }

    fn matches_condition(
        &self,
        condition: &InterceptCondition,
        request: &InterceptRequest,
        input_str: &str,
    ) -> bool {
        match condition {
            InterceptCondition::Pattern { pattern } => regex::Regex::new(pattern)
                .map(|re| re.is_match(input_str))
                .unwrap_or(false),
            InterceptCondition::Path { glob } => {
                let file_path = extract_file_path(request);
                match file_path {
                    Some(path) => glob_match(glob, &path),
                    None => false,
                }
            }
            InterceptCondition::Command { pattern } => {
                let command = extract_command(request);
                match command {
                    Some(cmd) => regex::Regex::new(pattern)
                        .map(|re| re.is_match(&cmd))
                        .unwrap_or(false),
                    None => false,
                }
            }
        }
    }
}

/// Extract file path from tool input (works for Write, Edit, Read tools).
fn extract_file_path(request: &InterceptRequest) -> Option<String> {
    request
        .tool_input
        .get("file_path")
        .or_else(|| request.tool_input.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract command string from tool input (works for Bash tool).
fn extract_command(request: &InterceptRequest) -> Option<String> {
    request
        .tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Simple glob matching (supports * and ** patterns).
fn glob_match(pattern: &str, path: &str) -> bool {
    // Convert glob to regex
    let mut regex_str = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => regex_str.push_str("\\."),
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume /
                        regex_str.push_str("(.+/)?");
                    } else {
                        regex_str.push_str(".*");
                    }
                } else {
                    regex_str.push_str("[^/]*");
                }
            }
            '?' => regex_str.push('.'),
            c if "()[]{}+^$|\\".contains(c) => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }
    regex::Regex::new(&format!("^{regex_str}$"))
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

/// Convert a storage Rule into an InterceptRule.
impl From<&crate::core::storage::Rule> for InterceptRule {
    fn from(r: &crate::core::storage::Rule) -> Self {
        let condition = match r.condition_type.as_str() {
            "pattern" => InterceptCondition::Pattern {
                pattern: r.condition_value.clone(),
            },
            "path" => InterceptCondition::Path {
                glob: r.condition_value.clone(),
            },
            "command" => InterceptCondition::Command {
                pattern: r.condition_value.clone(),
            },
            _ => InterceptCondition::Pattern {
                pattern: r.condition_value.clone(),
            },
        };

        InterceptRule {
            id: r.id.clone(),
            description: r.description.clone(),
            tools: r.tools.clone(),
            condition,
        }
    }
}

/// Default interception rules shipped with Synodic.
pub fn default_rules() -> Vec<InterceptRule> {
    vec![
        InterceptRule {
            id: "destructive-git".to_string(),
            description: "Block destructive git operations on protected branches".to_string(),
            tools: vec!["Bash".to_string()],
            condition: InterceptCondition::Command {
                pattern: r"git\s+(reset\s+--hard|push\s+--force|push\s+-f|clean\s+-fd)\b"
                    .to_string(),
            },
        },
        InterceptRule {
            id: "secrets-in-args".to_string(),
            description: "Block tool calls containing potential secrets".to_string(),
            tools: vec![],
            condition: InterceptCondition::Pattern {
                pattern: r"(?i)(api[_-]?key|secret[_-]?key|password|token)\s*[=:]\s*\S{8,}"
                    .to_string(),
            },
        },
        InterceptRule {
            id: "writes-outside-project".to_string(),
            description: "Block file writes outside the project root".to_string(),
            tools: vec!["Write".to_string(), "Edit".to_string()],
            condition: InterceptCondition::Path {
                glob: "/etc/**".to_string(),
            },
        },
        InterceptRule {
            id: "writes-to-system".to_string(),
            description: "Block file writes to system directories".to_string(),
            tools: vec!["Write".to_string(), "Edit".to_string()],
            condition: InterceptCondition::Path {
                glob: "/usr/**".to_string(),
            },
        },
        InterceptRule {
            id: "dangerous-rm".to_string(),
            description: "Block rm -rf with root or home paths".to_string(),
            tools: vec!["Bash".to_string()],
            condition: InterceptCondition::Command {
                pattern: r"rm\s+-[rR]f?\s+(/\s|/$|~/|~\s|~$|\$HOME)".to_string(),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> InterceptEngine {
        InterceptEngine::new(default_rules())
    }

    #[test]
    fn test_allow_normal_write() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/home/user/project/src/main.rs",
                "content": "fn main() {}"
            }),
        });
        assert_eq!(resp.decision, "allow");
        assert!(resp.reason.is_none());
    }

    #[test]
    fn test_block_write_to_etc() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/etc/passwd",
                "content": "malicious"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("writes-outside-project"));
    }

    #[test]
    fn test_block_write_to_usr() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/usr/local/bin/exploit",
                "content": "malicious"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("writes-to-system"));
    }

    #[test]
    fn test_block_destructive_git() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({
                "command": "git reset --hard HEAD~5"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("destructive-git"));
    }

    #[test]
    fn test_block_force_push() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({
                "command": "git push --force origin main"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("destructive-git"));
    }

    #[test]
    fn test_allow_normal_git() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({
                "command": "git push -u origin feature-branch"
            }),
        });
        assert_eq!(resp.decision, "allow");
    }

    #[test]
    fn test_block_secrets() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/home/user/project/.env",
                "content": "API_KEY=sk-1234567890abcdef"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("secrets-in-args"));
    }

    #[test]
    fn test_block_dangerous_rm() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({
                "command": "rm -rf /"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("dangerous-rm"));
    }

    #[test]
    fn test_allow_safe_rm() {
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({
                "command": "rm -rf target/debug"
            }),
        });
        assert_eq!(resp.decision, "allow");
    }

    #[test]
    fn test_rule_tool_filtering() {
        // The "writes-outside-project" rule only applies to Write/Edit,
        // not to Bash or Read
        let resp = engine().evaluate(&InterceptRequest {
            tool_name: "Read".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/etc/passwd"
            }),
        });
        assert_eq!(resp.decision, "allow");
    }

    #[test]
    fn test_custom_rules() {
        let rules = vec![InterceptRule {
            id: "no-prod-config".to_string(),
            description: "Block modifications to production config".to_string(),
            tools: vec!["Write".to_string(), "Edit".to_string()],
            condition: InterceptCondition::Path {
                glob: "**/config/production.*".to_string(),
            },
        }];
        let engine = InterceptEngine::new(rules);

        let resp = engine.evaluate(&InterceptRequest {
            tool_name: "Edit".to_string(),
            tool_input: serde_json::json!({
                "file_path": "/home/user/app/config/production.yml"
            }),
        });
        assert_eq!(resp.decision, "block");
        assert_eq!(resp.rule.as_deref(), Some("no-prod-config"));
    }

    #[test]
    fn test_response_serialization() {
        let allow = InterceptResponse::allow();
        let json = serde_json::to_string(&allow).unwrap();
        assert_eq!(json, r#"{"decision":"allow"}"#);

        let block = InterceptResponse::block("test reason", "test-rule");
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"decision\":\"block\""));
        assert!(json.contains("\"reason\":\"test reason\""));
        assert!(json.contains("\"rule\":\"test-rule\""));
    }
}
