//! Trigger-kind registry manifest — spec #237 / parent #236.
//!
//! This module is the **single source of truth** for what trigger kinds
//! exist in the factory and their metadata. It mirrors the event manifest
//! pattern from Lever E (`events.rs` / spec #150).
//!
//! Each row declares:
//! - `kind_tag` — snake-case key, matching
//!   [`onsager_spine::TriggerKind::kind_tag`] and the value persisted in
//!   `workflows.trigger_kind`.
//! - `producer` — subsystem that fires this trigger onto the bus.
//! - `category` — high-level taxonomy slot (`event` / `schedule` /
//!   `request` / `manual`); the dashboard uses it to group kinds.
//! - `ui_kind` — drives the per-kind config form shape.
//! - `description` — one-line human-readable summary for the UI.
//!
//! ## Update process
//!
//! Adding a `TriggerKind` variant requires appending a row here in the
//! same PR. `cargo xtask check-events` (extended in #237 for triggers)
//! enforces that every variant has a manifest row and every manifest row
//! is wired to a producer.
//!
//! Exposed at `GET /api/registry/triggers` so the dashboard's
//! `<TriggerKindPicker>` can render the catalog without hardcoding it.

use serde::Serialize;

use crate::events::Subsystem;

/// High-level trigger taxonomy. Dashboards use this to group kinds; the
/// factory runtime treats them all the same — every fired trigger lands
/// on the spine as `trigger.fired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCategory {
    /// External events delivered as webhooks (e.g. GitHub).
    Event,
    /// Time-based fires (cron / interval) — reserved for v2.
    Schedule,
    /// Caller-initiated requests via dashboard or CLI — reserved for v2.
    Request,
    /// Explicit human "go" signals — reserved for v2.
    Manual,
}

/// UI form shape used by the dashboard `<TriggerKindPicker>` to render
/// per-kind config. Today there is exactly one — `Webhook` — which
/// displays the existing GitHub-issue config form. Future kinds (cron,
/// interval) will introduce additional shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerUiKind {
    Webhook,
}

/// One row of the trigger-kind registry manifest.
#[derive(Debug, Clone, Serialize)]
pub struct TriggerDefinition {
    /// Snake-case wire key, matching `TriggerKind::kind_tag()`.
    pub kind_tag: &'static str,
    /// Subsystem that fires this trigger onto the spine
    /// (`trigger.fired` event producer).
    pub producer: Subsystem,
    /// Taxonomy slot for dashboard grouping.
    pub category: TriggerCategory,
    /// UI form shape for per-kind config.
    pub ui_kind: TriggerUiKind,
    /// One-line description rendered by the trigger picker.
    pub description: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerManifest {
    pub triggers: &'static [TriggerDefinition],
}

impl TriggerManifest {
    /// Lookup a manifest row by its snake-case `kind_tag`. Returns
    /// `None` for unknown kinds; callers can use this to validate
    /// inserts at the route boundary (the runtime registry check that
    /// replaces the old `CHECK (trigger_kind IN (...))` constraint).
    pub fn lookup(&self, kind_tag: &str) -> Option<&TriggerDefinition> {
        self.triggers.iter().find(|t| t.kind_tag == kind_tag)
    }
}

/// The canonical trigger-kind registry. Every variant in
/// `onsager_spine::TriggerKind` must have a row here.
pub const TRIGGERS: TriggerManifest = TriggerManifest {
    triggers: &[TriggerDefinition {
        kind_tag: "github_issue_webhook",
        producer: Subsystem::Stiglab,
        category: TriggerCategory::Event,
        ui_kind: TriggerUiKind::Webhook,
        description: "Fires when a GitHub issue is labeled with the configured label.",
    }],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_one_entry_per_known_kind() {
        let kinds: Vec<&str> = TRIGGERS.triggers.iter().map(|t| t.kind_tag).collect();
        assert!(kinds.contains(&"github_issue_webhook"));
        // Every kind_tag must be unique.
        let mut seen = std::collections::HashSet::new();
        for k in &kinds {
            assert!(seen.insert(*k), "duplicate kind_tag in manifest: {k}");
        }
    }

    #[test]
    fn lookup_returns_known_entry() {
        let entry = TRIGGERS.lookup("github_issue_webhook").unwrap();
        assert_eq!(entry.producer, Subsystem::Stiglab);
        assert_eq!(entry.category, TriggerCategory::Event);
        assert_eq!(entry.ui_kind, TriggerUiKind::Webhook);
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(TRIGGERS.lookup("polling").is_none());
    }

    #[test]
    fn manifest_serializes_to_expected_shape() {
        let v = serde_json::to_value(&TRIGGERS).unwrap();
        let triggers = v["triggers"].as_array().unwrap();
        assert_eq!(triggers.len(), TRIGGERS.triggers.len());
        let first = &triggers[0];
        assert_eq!(first["kind_tag"], "github_issue_webhook");
        assert_eq!(first["producer"], "stiglab");
        assert_eq!(first["category"], "event");
        assert_eq!(first["ui_kind"], "webhook");
        assert!(first["description"].as_str().is_some());
    }
}
