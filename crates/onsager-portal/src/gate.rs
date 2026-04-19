//! Synodic gate client (Phase 2).
//!
//! Wraps the `POST /api/gate` endpoint synodic exposes, plus the rule-less
//! short-circuit: the portal asks synodic if any rules apply to a project's
//! `(workspace, kind)` pair, and synthesises an `Allow` verdict locally
//! when the answer is none. That way the event shape is uniform whether or
//! not a tenant has actually authored gate rules — ising and the governance
//! feed never need a "is gating wired up?" branch.

use serde::{Deserialize, Serialize};

/// Outcome the portal acts on. Mirrors the `synodic` `GateVerdict` enum but
/// flattens the parameters the portal actually uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny { reason: String },
    Modify,
    Escalate { reason: String },
}

impl Verdict {
    pub fn as_summary(&self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Deny { .. } => "deny",
            Verdict::Modify => "modify",
            Verdict::Escalate { .. } => "escalate",
        }
    }
}

/// Minimal context the portal forwards to synodic.
#[derive(Debug, Clone, Serialize)]
pub struct GateInput {
    pub artifact_id: String,
    pub artifact_kind: String,
    pub current_state: String,
    pub head_sha: String,
}

#[derive(Debug, Clone)]
pub struct GateClient {
    http: reqwest::Client,
    base: Option<String>,
}

impl GateClient {
    pub fn new(base: Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("onsager-portal/0.1")
                .build()
                .expect("reqwest client"),
            base,
        }
    }

    /// Evaluate a gate. Returns `Allow` synthetically when no synodic URL is
    /// configured (dev / rule-less projects) or when synodic returns no
    /// applicable rules. Errors short-circuit to the configured fail policy
    /// — for v1 we always escalate so a synodic outage never silently
    /// approves changes.
    pub async fn evaluate(&self, input: &GateInput) -> Verdict {
        let Some(base) = self.base.as_ref() else {
            return Verdict::Allow;
        };
        let url = format!("{}/api/gate", base.trim_end_matches('/'));
        let body = serde_json::json!({
            "context": {
                "gate_point": "state_transition",
                "artifact_id": input.artifact_id,
                "artifact_kind": input.artifact_kind,
                "current_state": input.current_state,
                "extra": { "head_sha": input.head_sha }
            },
            "proposed_action": {
                "description": "PR commit gate",
                "payload": serde_json::json!({})
            }
        });
        let response = match self.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "synodic gate unreachable; escalating");
                return Verdict::Escalate {
                    reason: "synodic gate unreachable".into(),
                };
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "synodic gate returned error");
            return Verdict::Escalate {
                reason: format!("synodic gate returned {}", response.status()),
            };
        }
        match response.json::<RawVerdict>().await {
            Ok(v) => v.into(),
            Err(e) => {
                tracing::warn!(error = %e, "synodic gate response could not be parsed");
                Verdict::Escalate {
                    reason: "synodic gate response unparseable".into(),
                }
            }
        }
    }
}

/// Wire-format mirror of `onsager_protocol::GateVerdict`. Kept local so the
/// portal doesn't pull in the protocol crate just for one DTO; if it ever
/// needs more shapes, it can adopt the protocol crate then.
#[derive(Debug, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
enum RawVerdict {
    Allow,
    Deny { reason: String },
    Modify,
    Escalate { context: EscalationContext },
}

#[derive(Debug, Deserialize)]
struct EscalationContext {
    #[serde(default)]
    reason: String,
}

impl From<RawVerdict> for Verdict {
    fn from(v: RawVerdict) -> Self {
        match v {
            RawVerdict::Allow => Verdict::Allow,
            RawVerdict::Deny { reason } => Verdict::Deny { reason },
            RawVerdict::Modify => Verdict::Modify,
            RawVerdict::Escalate { context } => Verdict::Escalate {
                reason: context.reason,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_url_returns_allow() {
        let client = GateClient::new(None);
        let v = client
            .evaluate(&GateInput {
                artifact_id: "art_pr_1".into(),
                artifact_kind: "pull_request".into(),
                current_state: "in_progress".into(),
                head_sha: "deadbeef".into(),
            })
            .await;
        assert_eq!(v, Verdict::Allow);
    }
}
