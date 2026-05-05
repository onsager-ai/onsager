//! Workflow preset registry (issue #81).
//!
//! A preset expands a simple id (e.g. `"github-issue-to-pr"`) into a concrete
//! trigger + stage chain on workflow creation. Presets are **code-defined**:
//! callers can't register new presets at runtime in v1. This keeps the
//! mobile/chat UI simple (user picks a preset from a short list) and defers
//! per-org user-defined presets to v2.

use serde_json::json;

use crate::core::workflow::GateKind;

/// Stage spec from a preset, before being persisted with `workflow_id` + `seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetStage {
    pub gate_kind: GateKind,
    pub params: serde_json::Value,
}

/// Expansion produced by a preset: the trigger kind tag plus the ordered
/// stage chain. The caller fills in per-trigger config (e.g. repo / label)
/// at workflow creation time.
#[derive(Debug, Clone)]
pub struct PresetExpansion {
    pub preset_id: String,
    /// Snake-case `kind_tag` of the trigger this preset expects, matching
    /// [`onsager_spine::TriggerKind::kind_tag`]. The caller supplies the
    /// per-kind config (repo / label) separately.
    pub trigger_kind_tag: &'static str,
    pub stages: Vec<PresetStage>,
}

/// Resolve a preset id to its expansion. `None` when the id isn't known.
///
/// v1 ships a single preset: `github-issue-to-pr` — an issue labeled with the
/// workflow's trigger label opens a PR via an agent session, then releases.
/// The stage chain here matches the parent spec (#79):
/// `agent-session (implement + push + open PR) → released`.
pub fn resolve_preset(id: &str) -> Option<PresetExpansion> {
    match id {
        "github-issue-to-pr" => Some(PresetExpansion {
            preset_id: id.to_string(),
            trigger_kind_tag: "github_issue_webhook",
            stages: vec![PresetStage {
                gate_kind: GateKind::AgentSession,
                params: json!({
                    "action": "implement-and-open-pr",
                    "producer_profile": "implementer",
                    "release_on_complete": true,
                }),
            }],
        }),
        _ => None,
    }
}

/// Ids of every shipped preset — used by the dashboard's preset picker and
/// by CRUD validation to reject unknown preset ids.
pub const PRESET_IDS: &[&str] = &["github-issue-to-pr"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_issue_to_pr_expands_as_specced() {
        let p = resolve_preset("github-issue-to-pr").expect("preset should resolve");
        assert_eq!(p.preset_id, "github-issue-to-pr");
        assert_eq!(p.trigger_kind_tag, "github_issue_webhook");
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].gate_kind, GateKind::AgentSession);
        assert_eq!(
            p.stages[0].params.get("action").and_then(|v| v.as_str()),
            Some("implement-and-open-pr")
        );
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(resolve_preset("nope").is_none());
    }

    #[test]
    fn preset_ids_all_resolve() {
        for id in PRESET_IDS {
            assert!(
                resolve_preset(id).is_some(),
                "PRESET_IDS lists unknown preset: {id}"
            );
        }
    }
}
