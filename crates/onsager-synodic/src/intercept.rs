use regex::Regex;
use serde::{Deserialize, Serialize};

/// A pending tool call to evaluate against governance rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptRequest {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
}

/// The result of evaluating a tool call against rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptResponse {
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

impl InterceptResponse {
    pub fn allow() -> Self {
        Self {
            decision: Decision::Allow,
            reason: None,
            rule: None,
        }
    }

    pub fn block(reason: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            decision: Decision::Block,
            reason: Some(reason.into()),
            rule: Some(rule.into()),
        }
    }
}

/// Allow or block decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Block,
}

/// A governance rule that can block specific tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptRule {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    pub condition: InterceptCondition,
}

/// Condition types for matching tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InterceptCondition {
    /// Regex pattern matched against the serialized tool input.
    Pattern { pattern: String },
    /// Glob pattern matched against file_path or path fields.
    Path { glob: String },
    /// Regex pattern matched against the command field.
    Command { pattern: String },
}

/// The intercept engine — evaluates tool calls against a set of rules.
pub struct InterceptEngine {
    rules: Vec<InterceptRule>,
}

impl InterceptEngine {
    pub fn new(rules: Vec<InterceptRule>) -> Self {
        Self { rules }
    }

    /// Create an engine with the 5 default governance rules.
    pub fn with_defaults() -> Self {
        Self::new(default_rules())
    }

    /// Evaluate a tool call against all rules. Returns the first block match,
    /// or allow if no rules match.
    pub fn evaluate(&self, request: &InterceptRequest) -> InterceptResponse {
        for rule in &self.rules {
            // Check tool filter
            if let Some(tools) = &rule.tools {
                if !tools.iter().any(|t| t == &request.tool_name) {
                    continue;
                }
            }

            if self.matches_condition(&rule.condition, request) {
                return InterceptResponse::block(&rule.description, &rule.id);
            }
        }
        InterceptResponse::allow()
    }

    /// Get the active rules.
    pub fn rules(&self) -> &[InterceptRule] {
        &self.rules
    }

    fn matches_condition(
        &self,
        condition: &InterceptCondition,
        request: &InterceptRequest,
    ) -> bool {
        match condition {
            InterceptCondition::Pattern { pattern } => {
                let input_str = serde_json::to_string(&request.tool_input).unwrap_or_default();
                Regex::new(pattern)
                    .map(|re| re.is_match(&input_str))
                    .unwrap_or(false)
            }
            InterceptCondition::Path { glob } => {
                if let Some(path) = extract_file_path(&request.tool_input) {
                    glob_match(glob, &path)
                } else {
                    false
                }
            }
            InterceptCondition::Command { pattern } => {
                if let Some(cmd) = extract_command(&request.tool_input) {
                    Regex::new(pattern)
                        .map(|re| re.is_match(&cmd))
                        .unwrap_or(false)
                } else {
                    false
                }
            }
        }
    }
}

/// Extract file_path or path from tool input JSON.
fn extract_file_path(input: &serde_json::Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract command from tool input JSON.
fn extract_command(input: &serde_json::Value) -> Option<String> {
    input
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Convert a glob pattern to a regex pattern.
fn glob_match(glob: &str, path: &str) -> bool {
    let mut regex_str = String::from("^");
    let mut chars = glob.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // ** matches any path segment
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        regex_str.push_str("(.*/)?");
                    } else {
                        regex_str.push_str(".*");
                    }
                } else {
                    regex_str.push_str("[^/]*");
                }
            }
            '?' => regex_str.push_str("[^/]"),
            '.' | '(' | ')' | '+' | '|' | '^' | '$' | '{' | '}' | '[' | ']' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }
    regex_str.push('$');

    Regex::new(&regex_str)
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

/// The 5 default governance rules — ported from synodic's harness-core.
pub fn default_rules() -> Vec<InterceptRule> {
    vec![
        InterceptRule {
            id: "destructive_git".to_string(),
            description: "Block destructive git operations (force push, hard reset, clean)"
                .to_string(),
            tools: Some(vec!["Bash".to_string()]),
            condition: InterceptCondition::Command {
                pattern: r"git\s+(push\s+.*--force|push\s+.*-f\b|reset\s+--hard|clean\s+-fd)"
                    .to_string(),
            },
        },
        InterceptRule {
            id: "secrets_in_args".to_string(),
            description: "Block tool calls that contain potential secrets in arguments".to_string(),
            tools: None,
            condition: InterceptCondition::Pattern {
                pattern: r"(?i)(password|secret|token|api[_-]?key)\s*[=:]".to_string(),
            },
        },
        InterceptRule {
            id: "etc_writes".to_string(),
            description: "Block writes to /etc/".to_string(),
            tools: Some(vec!["Write".to_string(), "Edit".to_string()]),
            condition: InterceptCondition::Path {
                glob: "/etc/**".to_string(),
            },
        },
        InterceptRule {
            id: "usr_writes".to_string(),
            description: "Block writes to /usr/".to_string(),
            tools: Some(vec!["Write".to_string(), "Edit".to_string()]),
            condition: InterceptCondition::Path {
                glob: "/usr/**".to_string(),
            },
        },
        InterceptRule {
            id: "dangerous_rm".to_string(),
            description: "Block dangerous rm commands (rm -rf / or ~)".to_string(),
            tools: Some(vec!["Bash".to_string()]),
            condition: InterceptCondition::Command {
                pattern: r"rm\s+-[rR]f\s+(/\s*$|/\s+|~\s*$|~/|~\s+|\$HOME)".to_string(),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> InterceptEngine {
        InterceptEngine::with_defaults()
    }

    #[test]
    fn test_normal_write_allowed() {
        let req = InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({"file_path": "/home/user/hello.py", "content": "print('hi')"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Allow);
    }

    #[test]
    fn test_etc_write_blocked() {
        let req = InterceptRequest {
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({"file_path": "/etc/passwd", "content": "bad"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
        assert_eq!(resp.rule.as_deref(), Some("etc_writes"));
    }

    #[test]
    fn test_usr_write_blocked() {
        let req = InterceptRequest {
            tool_name: "Edit".to_string(),
            tool_input: serde_json::json!({"file_path": "/usr/bin/python", "content": "bad"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
        assert_eq!(resp.rule.as_deref(), Some("usr_writes"));
    }

    #[test]
    fn test_destructive_git_blocked() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push --force origin main"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
        assert_eq!(resp.rule.as_deref(), Some("destructive_git"));
    }

    #[test]
    fn test_git_push_force_short_flag() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push -f origin main"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
    }

    #[test]
    fn test_git_reset_hard_blocked() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git reset --hard HEAD~1"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
    }

    #[test]
    fn test_normal_git_allowed() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "git push -u origin main"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Allow);
    }

    #[test]
    fn test_secrets_blocked() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "curl -H 'api_key=sk-1234' https://api.example.com"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
        assert_eq!(resp.rule.as_deref(), Some("secrets_in_args"));
    }

    #[test]
    fn test_dangerous_rm_blocked() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "rm -rf /"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Block);
        assert_eq!(resp.rule.as_deref(), Some("dangerous_rm"));
    }

    #[test]
    fn test_safe_rm_allowed() {
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "rm -rf /tmp/build"}),
        };
        let resp = engine().evaluate(&req);
        assert_eq!(resp.decision, Decision::Allow);
    }

    #[test]
    fn test_tool_specific_filtering() {
        // etc_writes only applies to Write and Edit, not Bash
        let req = InterceptRequest {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"file_path": "/etc/hosts"}),
        };
        let resp = engine().evaluate(&req);
        // Should be allowed because Bash is not in the tool filter for etc_writes
        assert_eq!(resp.decision, Decision::Allow);
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("/etc/**", "/etc/passwd"));
        assert!(glob_match("/etc/**", "/etc/nginx/nginx.conf"));
        assert!(!glob_match("/etc/**", "/home/user/file"));
        assert!(glob_match("/usr/**", "/usr/bin/python"));
        assert!(!glob_match("/usr/**", "/var/log/syslog"));
    }
}
