//! Typed GitHub webhook event vocabulary.
//!
//! **Status:** stub. The full host-agnostic translation
//! (`code.pr_merged`, `code.issue_commented`, …) lands together with
//! the spine event registry from #150 — see #220 Sub-issue C. Today
//! the portal still parses `serde_json::Value` payloads in
//! `handlers/`; this module is the home those handlers will move into.
//!
//! What lives here now: the variant skeleton + the `to_spine_events`
//! translator signature. Wiring real consumers means filling each
//! variant's body and adding the matching `FactoryEventKind` rows.

use onsager_spine::FactoryEvent;
use serde::{Deserialize, Serialize};

/// Coarse classification of the webhook event types Onsager already
/// routes today. Add variants as new routing rules land — the goal is
/// one variant per `code.*` event the spine carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebhookEvent {
    IssuesLabeled {
        owner: String,
        repo: String,
        issue_number: u64,
        label: String,
    },
    PullRequestClosedMerged {
        owner: String,
        repo: String,
        number: u64,
        merge_commit_sha: Option<String>,
    },
    CheckSuiteCompleted {
        owner: String,
        repo: String,
        head_sha: String,
        conclusion: Option<String>,
    },
    CheckRunCompleted {
        owner: String,
        repo: String,
        head_sha: String,
        name: String,
        conclusion: Option<String>,
    },
    /// Catch-all for delivered events whose typed shape hasn't landed
    /// yet. Lets the portal log + ack without dropping deliveries.
    Other { event_type: String },
}

/// Translate a typed webhook event into the host-agnostic spine
/// events it should produce.
///
/// Returns `Vec` because one webhook can fan out to multiple spine
/// events (e.g. PR merge → both `code.pr_merged` and a workflow
/// activation signal). Empty vec is a valid no-op for events we
/// receive but don't yet route.
///
/// **Stub:** today this returns `vec![]` for every variant. The
/// registry-backed event types (#150) are the prerequisite for
/// filling it in — see #220 Sub-issue C.
pub fn to_spine_events(_event: &WebhookEvent) -> Vec<FactoryEvent> {
    Vec::new()
}
