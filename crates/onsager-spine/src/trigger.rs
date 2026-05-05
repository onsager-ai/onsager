//! Canonical workflow trigger type — Lever-E-style foundation for spec
//! #237 (parent #236). This module is the **single source of truth** for
//! what kinds of triggers exist in the factory and the per-kind config
//! they carry.
//!
//! The type doubles as wire format: it serializes to the JSON shape stored
//! in `workflows.trigger_config` (the `kind_tag` lives in
//! `workflows.trigger_kind`, the rest of the variant fields in the JSONB
//! column). Persistence layers reconstruct a [`TriggerKind`] from that
//! `(kind, config)` pair via [`TriggerKind::from_storage`].
//!
//! Adding a new variant is a three-step contract that the registry
//! manifest at `crates/onsager-registry/src/triggers.rs` enforces:
//! 1. Add a variant here with its config fields.
//! 2. Add a row to the registry manifest with the snake-case `kind_tag`.
//! 3. Wire a producer + consumer (or tag the manifest row `audit_only`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A workflow-runtime trigger and its configuration.
///
/// `serde` representation is `tag = "kind"` with snake_case keys, matching
/// the persisted `workflows.trigger_kind` column and `FactoryEventKind`'s
/// wire form. New variants append at the end; do not reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerKind {
    /// A GitHub `issues.labeled` webhook whose label matches `label`.
    /// `repo` is the `"owner/name"` slug.
    GithubIssueWebhook { repo: String, label: String },

    // -- Schedule (#238) ----------------------------------------------------
    /// Fire on a cron schedule. `expression` is a 5- or 6-field cron string
    /// (minute, hour, day-of-month, month, day-of-week, optional seconds).
    /// `timezone` is an IANA name (e.g. `"UTC"`, `"America/Los_Angeles"`);
    /// defaults to UTC when absent.
    Cron {
        expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },

    /// Fire once after a fixed delay measured from `anchor`. v1's only
    /// anchor is the workflow's activation time; future anchors include
    /// "delay relative to a received spine event".
    Delay {
        seconds: u64,
        #[serde(default)]
        anchor: DelayAnchor,
    },

    /// Fire periodically every `period_seconds`. Catch-up policy after an
    /// outage is "skip missed, fire only the next due firing" (per #238
    /// resolution); replay-of-missed is a per-workflow follow-up.
    Interval { period_seconds: u64 },

    // -- Event (#239) -------------------------------------------------------
    /// Fire when the spine emits a `FactoryEventKind` whose `type` matches
    /// `event_kind`. `filter` is an optional JSON-shape predicate against
    /// the event payload (equality + `$.path` only).
    SpineEvent {
        event_kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<JsonFilter>,
    },

    /// Fire when a Postgres `NOTIFY <channel>` arrives. `filter` matches
    /// against the parsed JSON payload of the notification (when present).
    PgNotify {
        channel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<JsonFilter>,
    },

    /// Fire when a row matching `where_clause` is inserted into `table`.
    /// The poller advances a per-workflow cursor in the
    /// `outbox_trigger_cursor` sidecar table; `where_clause` is appended
    /// to a parameterized poll query.
    OutboxRow { table: String, where_clause: String },

    // -- Manual (#241) ------------------------------------------------------
    /// Fire on demand from a UI button or CLI command. `name` is the
    /// workflow-author's label for the button (rendered as the button
    /// text in the UI). The current `workflows` schema persists exactly
    /// one trigger per workflow; if the umbrella later supports multiple
    /// triggers per workflow, multiple `Manual { name }` entries would
    /// render as separate buttons.
    Manual { name: String },

    /// Re-emit the payload of a past `TriggerFired` event by event id.
    /// Replay is registered as a workflow trigger so the replay primitive
    /// shares the audit + permission shape with manual fires; #168's
    /// backfill UI consumes it for past GitHub-issue events.
    Replay { source_event_id: String },
}

/// Anchor for [`TriggerKind::Delay`]. v1 only supports
/// `WorkflowActivatedAt` (delay measured from the workflow row's
/// `created_at` / `last_fired_at` baseline). Future variants:
/// `EventReceivedAt(EventKind)` — needs the event-trigger category to
/// land first to define "when was an event received".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "anchor", rename_all = "snake_case")]
pub enum DelayAnchor {
    #[default]
    WorkflowActivatedAt,
}

/// Simple JSON-shape predicate for `SpineEvent` and `PgNotify` triggers.
///
/// `equals` carries top-level field or `$.path` equality assertions:
/// - `{"workspace_id": "ws_x"}` matches when the payload's `workspace_id`
///   field equals `"ws_x"`.
/// - `{"$.payload.repo": "owner/name"}` matches when the JSONPath-ish
///   dotted lookup hits the same string.
///
/// Full JSONata is out of scope (per #239 resolution) — the simple form
/// covers the known use cases and keeps the evaluator small.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct JsonFilter {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub equals: BTreeMap<String, serde_json::Value>,
}

impl JsonFilter {
    /// Returns `true` when every entry in `equals` matches the corresponding
    /// path in `value`. An empty filter (no entries) matches everything.
    /// Unknown paths are treated as a non-match.
    pub fn matches(&self, value: &serde_json::Value) -> bool {
        self.equals.iter().all(|(k, expected)| {
            let actual = lookup_json_path(value, k);
            actual.map(|v| v == expected).unwrap_or(false)
        })
    }
}

/// Walk a dotted path through a JSON value. Accepts both `"foo"` (top-level
/// field) and `"$.foo.bar"` / `"foo.bar"` (nested dotted lookup). Returns
/// `None` if any segment is missing or not an object.
fn lookup_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let stripped = path.strip_prefix("$.").unwrap_or(path);
    let mut cur = value;
    for segment in stripped.split('.') {
        if segment.is_empty() {
            continue;
        }
        cur = cur.get(segment)?;
    }
    Some(cur)
}

impl TriggerKind {
    /// Stable snake-case key for the variant; matches the value stored in
    /// `workflows.trigger_kind` and the `kind_tag` column in the trigger
    /// registry manifest.
    pub const fn kind_tag(&self) -> &'static str {
        match self {
            TriggerKind::GithubIssueWebhook { .. } => "github_issue_webhook",
            TriggerKind::Cron { .. } => "cron",
            TriggerKind::Delay { .. } => "delay",
            TriggerKind::Interval { .. } => "interval",
            TriggerKind::SpineEvent { .. } => "spine_event",
            TriggerKind::PgNotify { .. } => "pg_notify",
            TriggerKind::OutboxRow { .. } => "outbox_row",
            TriggerKind::Manual { .. } => "manual",
            TriggerKind::Replay { .. } => "replay",
        }
    }
}

/// Errors returned by [`TriggerKind::from_storage`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TriggerStorageError {
    #[error("unknown trigger kind: {0}")]
    UnknownKind(String),
    #[error("trigger config for {kind} failed to parse: {message}")]
    InvalidConfig { kind: &'static str, message: String },
}

impl TriggerKind {
    /// Reconstruct a [`TriggerKind`] from the persisted `(kind_tag, config)`
    /// split. The kind tag selects the variant; the JSONB blob supplies
    /// its config fields. Symmetric to [`TriggerKind::to_storage`].
    pub fn from_storage(
        kind_tag: &str,
        config: &serde_json::Value,
    ) -> Result<Self, TriggerStorageError> {
        // Resolve the static kind tag *first* so an unknown kind always
        // surfaces as `UnknownKind`, regardless of whether the stored
        // config happens to be malformed. Otherwise an unknown kind +
        // a non-object config would be reported as a generic
        // "InvalidConfig { kind: \"unknown\" }" — the unknown-kind
        // problem hidden by a config-shape error.
        let static_tag = static_kind_tag(kind_tag)
            .ok_or_else(|| TriggerStorageError::UnknownKind(kind_tag.to_string()))?;

        // Re-attach the kind discriminant and run serde — keeps the
        // per-variant parsing in one place rather than duplicating a
        // hand-written match for every new variant.
        let mut tagged = config.clone();
        if let Some(obj) = tagged.as_object_mut() {
            obj.insert(
                "kind".to_string(),
                serde_json::Value::String(kind_tag.to_string()),
            );
        } else if tagged.is_null() {
            tagged = serde_json::json!({ "kind": kind_tag });
        } else {
            return Err(TriggerStorageError::InvalidConfig {
                kind: static_tag,
                message: "trigger_config must be a JSON object".into(),
            });
        }
        serde_json::from_value::<TriggerKind>(tagged).map_err(|e| {
            TriggerStorageError::InvalidConfig {
                kind: static_tag,
                message: e.to_string(),
            }
        })
    }

    /// Split into the persisted `(kind_tag, config)` shape.
    /// `config` is the JSON object stored in `workflows.trigger_config`.
    pub fn to_storage(&self) -> (&'static str, serde_json::Value) {
        let mut value = serde_json::to_value(self).expect("TriggerKind serializes");
        // Strip the `kind` discriminant — it lives in the column, not the
        // JSONB. Symmetric with `from_storage`.
        if let Some(obj) = value.as_object_mut() {
            obj.remove("kind");
        }
        (self.kind_tag(), value)
    }
}

/// Map a runtime kind tag back to the static `&'static str` referenced by
/// each `TriggerKind` variant. Returning the static form keeps
/// [`TriggerStorageError::InvalidConfig`]'s `kind` field a static string,
/// which is what the registry manifest's `kind_tag` field also uses.
fn static_kind_tag(kind_tag: &str) -> Option<&'static str> {
    match kind_tag {
        "github_issue_webhook" => Some("github_issue_webhook"),
        "cron" => Some("cron"),
        "delay" => Some("delay"),
        "interval" => Some("interval"),
        "spine_event" => Some("spine_event"),
        "pg_notify" => Some("pg_notify"),
        "outbox_row" => Some("outbox_row"),
        "manual" => Some("manual"),
        "replay" => Some("replay"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn variant_serializes_with_kind_tag() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "ai".into(),
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["kind"], "github_issue_webhook");
        assert_eq!(v["repo"], "owner/name");
        assert_eq!(v["label"], "ai");
    }

    #[test]
    fn variant_round_trips_through_serde() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "ai".into(),
        };
        let v = serde_json::to_value(&t).unwrap();
        let back: TriggerKind = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn kind_tag_is_snake_case() {
        let t = TriggerKind::GithubIssueWebhook {
            repo: "x".into(),
            label: "y".into(),
        };
        assert_eq!(t.kind_tag(), "github_issue_webhook");
    }

    #[test]
    fn from_storage_reconstructs_variant() {
        let cfg = json!({"repo": "owner/name", "label": "ai"});
        let t = TriggerKind::from_storage("github_issue_webhook", &cfg).unwrap();
        assert_eq!(
            t,
            TriggerKind::GithubIssueWebhook {
                repo: "owner/name".into(),
                label: "ai".into(),
            }
        );
    }

    #[test]
    fn from_storage_rejects_unknown_kind() {
        let err = TriggerKind::from_storage("polling", &json!({})).unwrap_err();
        assert!(matches!(err, TriggerStorageError::UnknownKind(ref s) if s == "polling"));
    }

    #[test]
    fn from_storage_rejects_missing_config_fields() {
        let err =
            TriggerKind::from_storage("github_issue_webhook", &json!({"repo": "a/b"})).unwrap_err();
        assert!(
            matches!(err, TriggerStorageError::InvalidConfig { kind, .. } if kind == "github_issue_webhook")
        );
    }

    #[test]
    fn to_storage_round_trips_through_from_storage() {
        let original = TriggerKind::GithubIssueWebhook {
            repo: "owner/name".into(),
            label: "planned".into(),
        };
        let (kind, cfg) = original.to_storage();
        let back = TriggerKind::from_storage(kind, &cfg).unwrap();
        assert_eq!(back, original);
    }

    // -- Schedule variants (#238) ------------------------------------------

    #[test]
    fn cron_round_trips_through_storage() {
        let t = TriggerKind::Cron {
            expression: "0 9 * * 1-5".into(),
            timezone: Some("America/Los_Angeles".into()),
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "cron");
        assert_eq!(cfg["expression"], "0 9 * * 1-5");
        assert_eq!(cfg["timezone"], "America/Los_Angeles");
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    #[test]
    fn cron_timezone_optional() {
        let t = TriggerKind::Cron {
            expression: "* * * * *".into(),
            timezone: None,
        };
        let (kind, cfg) = t.to_storage();
        assert!(cfg.get("timezone").is_none());
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    #[test]
    fn delay_round_trips_through_storage() {
        let t = TriggerKind::Delay {
            seconds: 30,
            anchor: DelayAnchor::WorkflowActivatedAt,
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "delay");
        assert_eq!(cfg["seconds"], 30);
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    #[test]
    fn interval_round_trips_through_storage() {
        let t = TriggerKind::Interval {
            period_seconds: 300,
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "interval");
        assert_eq!(cfg["period_seconds"], 300);
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    // -- Event variants (#239) ---------------------------------------------

    #[test]
    fn spine_event_round_trips_with_filter() {
        let mut equals = BTreeMap::new();
        equals.insert("workspace_id".into(), json!("ws_x"));
        let t = TriggerKind::SpineEvent {
            event_kind: "forge.shaping_dispatched".into(),
            filter: Some(JsonFilter { equals }),
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "spine_event");
        let back = TriggerKind::from_storage(kind, &cfg).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn pg_notify_round_trips_through_storage() {
        let t = TriggerKind::PgNotify {
            channel: "factory_signal".into(),
            filter: None,
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "pg_notify");
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    #[test]
    fn outbox_row_round_trips_through_storage() {
        let t = TriggerKind::OutboxRow {
            table: "artifact_outbox".into(),
            where_clause: "state = 'sealed'".into(),
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "outbox_row");
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    // -- Manual variants (#241) --------------------------------------------

    #[test]
    fn manual_round_trips_through_storage() {
        let t = TriggerKind::Manual {
            name: "rerun".into(),
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "manual");
        assert_eq!(cfg["name"], "rerun");
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    #[test]
    fn replay_round_trips_through_storage() {
        let t = TriggerKind::Replay {
            source_event_id: "9001".into(),
        };
        let (kind, cfg) = t.to_storage();
        assert_eq!(kind, "replay");
        assert_eq!(cfg["source_event_id"], "9001");
        assert_eq!(TriggerKind::from_storage(kind, &cfg).unwrap(), t);
    }

    // -- JsonFilter --------------------------------------------------------

    #[test]
    fn json_filter_empty_matches_anything() {
        let f = JsonFilter::default();
        assert!(f.matches(&json!({})));
        assert!(f.matches(&json!({"a": 1})));
    }

    #[test]
    fn json_filter_top_level_equality() {
        let mut equals = BTreeMap::new();
        equals.insert("workspace_id".into(), json!("ws_x"));
        let f = JsonFilter { equals };
        assert!(f.matches(&json!({"workspace_id": "ws_x", "extra": 1})));
        assert!(!f.matches(&json!({"workspace_id": "ws_y"})));
        assert!(!f.matches(&json!({})));
    }

    #[test]
    fn json_filter_dotted_path() {
        let mut equals = BTreeMap::new();
        equals.insert("$.payload.repo".into(), json!("a/b"));
        let f = JsonFilter { equals };
        assert!(f.matches(&json!({"payload": {"repo": "a/b"}})));
        assert!(!f.matches(&json!({"payload": {"repo": "x/y"}})));
        assert!(!f.matches(&json!({"payload": {}})));
    }

    #[test]
    fn json_filter_path_without_dollar_prefix_works() {
        let mut equals = BTreeMap::new();
        equals.insert("payload.repo".into(), json!("a/b"));
        let f = JsonFilter { equals };
        assert!(f.matches(&json!({"payload": {"repo": "a/b"}})));
    }

    #[test]
    fn from_storage_rejects_invalid_cron_config() {
        let err = TriggerKind::from_storage("cron", &json!({})).unwrap_err();
        assert!(matches!(err, TriggerStorageError::InvalidConfig { kind, .. } if kind == "cron"));
    }

    #[test]
    fn from_storage_rejects_invalid_interval_config() {
        let err =
            TriggerKind::from_storage("interval", &json!({"period_seconds": "five"})).unwrap_err();
        assert!(
            matches!(err, TriggerStorageError::InvalidConfig { kind, .. } if kind == "interval")
        );
    }

    #[test]
    fn from_storage_rejects_non_object_config() {
        let err = TriggerKind::from_storage("cron", &json!("not-an-object")).unwrap_err();
        assert!(matches!(err, TriggerStorageError::InvalidConfig { .. }));
    }
}
