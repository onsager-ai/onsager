//! The product's nouns as plain serde types — closed enums, no trait
//! objects, no registry (ADR 0029: the open executor catalog returns
//! if third-party executors ever become real).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What fires a workflow. M2 adds the GitHub-shaped kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerKind {
    /// Fired by `POST /api/workflows/:id/fire`.
    Manual { name: String },
}

/// One stage of a workflow definition. Execution is "iterate the
/// list" — the v1 three-step plan compiler collapsed into a loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageKind {
    /// Run a claude-CLI agent session. The session emits artifacts via
    /// the MCP `emit_artifact` tool; output text is recorded on the
    /// session, not parsed for deliverables.
    Agent {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        system_prompt: Option<String>,
        user_prompt: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Workflow {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    #[sqlx(json)]
    pub trigger: TriggerKind,
    #[sqlx(json)]
    pub definition: Vec<StageKind>,
    pub active: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Run {
    pub id: String,
    pub workspace_id: String,
    pub workflow_id: String,
    pub status: String,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Provenance rides every artifact from day one (identity commitment 2,
/// carried through ADR 0029). The Verify upgrade path returns when a
/// verifier exists; until then everything an agent makes is `Uncertain`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    Uncertain { source: String },
    Deterministic { source: String },
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Artifact {
    pub id: String,
    pub workspace_id: String,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub content_uri: Option<String>,
    pub content_checksum: Option<String>,
    pub external_ref: Option<serde_json::Value>,
    #[sqlx(json)]
    pub provenance: Provenance,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_and_stage_wire_shapes() {
        let t: TriggerKind = serde_json::from_value(serde_json::json!(
            {"kind": "manual", "name": "go"}
        ))
        .unwrap();
        assert_eq!(t, TriggerKind::Manual { name: "go".into() });

        let s: StageKind = serde_json::from_value(serde_json::json!(
            {"kind": "agent", "user_prompt": "do it", "system_prompt": "be brief"}
        ))
        .unwrap();
        let StageKind::Agent {
            model,
            system_prompt,
            user_prompt,
        } = s;
        assert_eq!(model, None);
        assert_eq!(system_prompt.as_deref(), Some("be brief"));
        assert_eq!(user_prompt, "do it");
    }

    #[test]
    fn provenance_wire_shape() {
        let p = Provenance::Uncertain {
            source: "agent".into(),
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({"kind": "uncertain", "source": "agent"})
        );
    }
}
