use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An extension event record as stored in the `events_ext` table.
/// Extension events have a namespaced type and a wide JSON payload,
/// allowing any component to publish signals without changing the core schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionEventRecord {
    pub id: i64,
    pub stream_id: String,
    pub namespace: String,
    pub event_type: String,
    pub data: serde_json::Value,
    pub metadata: serde_json::Value,
    pub ref_event_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl ExtensionEventRecord {
    /// Returns the fully qualified event type (e.g., "synodic.policy.denied").
    pub fn full_type(&self) -> String {
        format!("{}.{}", self.namespace, self.event_type)
    }
}
