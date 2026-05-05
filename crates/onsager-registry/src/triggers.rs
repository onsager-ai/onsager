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

/// High-level trigger taxonomy (per #236). Dashboards use this to group
/// kinds; the factory runtime treats them all the same — every fired
/// trigger lands on the spine as `trigger.fired`.
///
/// The split mirrors the spec's four categories:
///
/// - **Event** — internal event-bus signals (`spine_event`, `pg_notify`,
///   `outbox_row`). Producer is always forge.
/// - **Schedule** — time-based fires (`cron`, `delay`, `interval`).
///   Producer is always forge.
/// - **Request** — external HTTP requests (webhooks: GitHub, Telegram,
///   …). Producer is the edge subsystem hosting the receiver (stiglab
///   today; portal once #222 lands).
/// - **Manual** — user-initiated fires (UI button, CLI, replay).
///   Producer is portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCategory {
    /// Internal event-bus signals (spine events, pg_notify channels,
    /// outbox rows).
    Event,
    /// Time-based fires (cron / delay / interval).
    Schedule,
    /// External HTTP requests — webhooks (GitHub today; Telegram etc.
    /// in the works).
    Request,
    /// User-initiated fires (UI button, CLI, replay).
    Manual,
}

/// UI form shape used by the dashboard `<TriggerKindPicker>` to render
/// per-kind config. Each shape maps to a concrete form layout in the
/// dashboard's trigger picker; new shapes land alongside new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerUiKind {
    /// Webhook receivers (the existing GitHub-issue form).
    Webhook,
    /// Cron-expression input with a human-readable preview.
    Cron,
    /// Single-shot delay (seconds + anchor selector).
    Delay,
    /// Recurring interval (period in seconds).
    Interval,
    /// Spine `FactoryEventKind` selector + optional JSON filter.
    SpineEvent,
    /// Postgres `NOTIFY` channel name + optional JSON filter.
    PgNotify,
    /// Outbox table name + SQL `WHERE` clause.
    Outbox,
    /// Manual-fire button label.
    Manual,
    /// Replay of a past `TriggerFired` by event id.
    Replay,
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
    triggers: &[
        TriggerDefinition {
            kind_tag: "github_issue_webhook",
            producer: Subsystem::Stiglab,
            // Category 3 per #236 — webhooks are external HTTP requests,
            // not internal event-bus signals. Putting them in `Event`
            // would force `check-triggers` to allow Stiglab as an Event
            // producer, weakening enforcement for true event triggers
            // like `spine_event`.
            category: TriggerCategory::Request,
            ui_kind: TriggerUiKind::Webhook,
            description: "Fires when a GitHub issue is labeled with the configured label.",
        },
        // -- Schedule (#238) -----------------------------------------------
        TriggerDefinition {
            kind_tag: "cron",
            producer: Subsystem::Forge,
            category: TriggerCategory::Schedule,
            ui_kind: TriggerUiKind::Cron,
            description: "Fires on a cron schedule (5- or 6-field expression, optional timezone).",
        },
        TriggerDefinition {
            kind_tag: "delay",
            producer: Subsystem::Forge,
            category: TriggerCategory::Schedule,
            ui_kind: TriggerUiKind::Delay,
            description: "Fires once after a fixed delay measured from the workflow's activation.",
        },
        TriggerDefinition {
            kind_tag: "interval",
            producer: Subsystem::Forge,
            category: TriggerCategory::Schedule,
            ui_kind: TriggerUiKind::Interval,
            description: "Fires periodically at a fixed interval (in seconds).",
        },
        // -- Event (#239) --------------------------------------------------
        TriggerDefinition {
            kind_tag: "spine_event",
            producer: Subsystem::Forge,
            category: TriggerCategory::Event,
            ui_kind: TriggerUiKind::SpineEvent,
            description: "Fires when a spine FactoryEventKind matches; optional JSON filter.",
        },
        TriggerDefinition {
            kind_tag: "pg_notify",
            producer: Subsystem::Forge,
            category: TriggerCategory::Event,
            ui_kind: TriggerUiKind::PgNotify,
            description: "Fires on a Postgres NOTIFY channel; optional JSON filter on the payload.",
        },
        TriggerDefinition {
            kind_tag: "outbox_row",
            producer: Subsystem::Forge,
            category: TriggerCategory::Event,
            ui_kind: TriggerUiKind::Outbox,
            description: "Polls an outbox table for new rows matching a WHERE clause.",
        },
        // -- Manual (#241) -------------------------------------------------
        TriggerDefinition {
            kind_tag: "manual",
            producer: Subsystem::Portal,
            category: TriggerCategory::Manual,
            ui_kind: TriggerUiKind::Manual,
            description: "Fires from a UI button or `onsager trigger fire` CLI command.",
        },
        TriggerDefinition {
            kind_tag: "replay",
            producer: Subsystem::Portal,
            category: TriggerCategory::Manual,
            ui_kind: TriggerUiKind::Replay,
            description: "Re-emits the payload of a past TriggerFired event by event id.",
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_one_entry_per_known_kind() {
        let kinds: Vec<&str> = TRIGGERS.triggers.iter().map(|t| t.kind_tag).collect();
        // Foundation kinds.
        assert!(kinds.contains(&"github_issue_webhook"));
        // Schedule (#238).
        assert!(kinds.contains(&"cron"));
        assert!(kinds.contains(&"delay"));
        assert!(kinds.contains(&"interval"));
        // Event (#239).
        assert!(kinds.contains(&"spine_event"));
        assert!(kinds.contains(&"pg_notify"));
        assert!(kinds.contains(&"outbox_row"));
        // Manual (#241).
        assert!(kinds.contains(&"manual"));
        assert!(kinds.contains(&"replay"));
        // Every kind_tag must be unique.
        let mut seen = std::collections::HashSet::new();
        for k in &kinds {
            assert!(seen.insert(*k), "duplicate kind_tag in manifest: {k}");
        }
    }

    #[test]
    fn manifest_categories_match_taxonomy() {
        // Schedule kinds.
        for k in ["cron", "delay", "interval"] {
            assert_eq!(
                TRIGGERS.lookup(k).unwrap().category,
                TriggerCategory::Schedule
            );
        }
        // Event kinds (internal event-bus signals only).
        for k in ["spine_event", "pg_notify", "outbox_row"] {
            assert_eq!(TRIGGERS.lookup(k).unwrap().category, TriggerCategory::Event);
        }
        // Request kinds — external HTTP webhooks.
        assert_eq!(
            TRIGGERS.lookup("github_issue_webhook").unwrap().category,
            TriggerCategory::Request
        );
        // Manual kinds.
        for k in ["manual", "replay"] {
            assert_eq!(
                TRIGGERS.lookup(k).unwrap().category,
                TriggerCategory::Manual
            );
        }
    }

    #[test]
    fn lookup_returns_known_entry() {
        let entry = TRIGGERS.lookup("github_issue_webhook").unwrap();
        assert_eq!(entry.producer, Subsystem::Stiglab);
        assert_eq!(entry.category, TriggerCategory::Request);
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
        assert_eq!(first["category"], "request");
        assert_eq!(first["ui_kind"], "webhook");
        assert!(first["description"].as_str().is_some());
    }
}
