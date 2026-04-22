//! Deliverable — the workflow-run output aggregate.
//!
//! A [`Deliverable`] is an open-schema map from artifact kind to the
//! `ArtifactId`s that make up a workflow run's product. It matches the two-level
//! mental model from issue #100:
//!
//! - **WorkflowRun** (process, transient): which graph, current node, history.
//! - **Deliverable** (product, durable): open map `Kind → Vec<ArtifactId>`.
//!
//! Per-kind merge rules live in `onsager-registry` (issue #102, `MergeRule`);
//! this crate is only the value object.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::artifact::ArtifactId;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Globally unique identifier for a [`Deliverable`]. Format: `dlv_<26-char-ulid>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeliverableId(String);

impl DeliverableId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        let ulid = ulid::Ulid::new().to_string();
        Self(format!("dlv_{ulid}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeliverableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier for the workflow run that produced a deliverable.
///
/// Stored as a string so callers can wrap either the stiglab run id or a
/// forge-side run id without forcing a cross-crate type dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowRunId(pub String);

impl WorkflowRunId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkflowRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Registry-backed artifact kind id, e.g. `"Issue"`, `"PR"`, `"Deployment"`.
///
/// Stored as a string; the registry (`onsager-registry::RegistryId`) is the
/// source of truth for which ids are known. Kept string-typed here so the
/// value-object crate does not depend on the registry crate.
#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KindId(pub String);

impl KindId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KindId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for KindId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for KindId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ---------------------------------------------------------------------------
// Deliverable
// ---------------------------------------------------------------------------

/// The aggregate output of a workflow run, keyed by artifact kind.
///
/// Entries are referential — the actual artifact records live in their own
/// storage; the deliverable only points at them. Kinds that accept multiple
/// concurrent artifacts (e.g. `Commit`) hold a `Vec<ArtifactId>`; kinds with
/// a single-artifact cardinality (e.g. `Issue`) still use a vec so the shape
/// is uniform, and the registered `MergeRule` governs what "add" means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: DeliverableId,
    pub workflow_run_id: WorkflowRunId,
    #[serde(default)]
    pub entries: BTreeMap<KindId, Vec<ArtifactId>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Deliverable {
    /// Create an empty deliverable bound to a workflow run.
    pub fn new(workflow_run_id: WorkflowRunId) -> Self {
        let now = Utc::now();
        Self {
            id: DeliverableId::generate(),
            workflow_run_id,
            entries: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Append an artifact id under a kind. De-duplicates on exact id match so
    /// replaying the same event is idempotent.
    pub fn add(&mut self, kind: KindId, artifact_id: ArtifactId) {
        let slot = self.entries.entry(kind).or_default();
        if !slot.contains(&artifact_id) {
            slot.push(artifact_id);
        }
        self.updated_at = Utc::now();
    }

    /// All artifact ids for a given kind, in insertion order.
    pub fn get(&self, kind: &KindId) -> &[ArtifactId] {
        self.entries.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Total number of artifact references across all kinds.
    pub fn len(&self) -> usize {
        self.entries.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.values().all(Vec::is_empty)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliverable_id_format() {
        let id = DeliverableId::generate();
        assert!(id.as_str().starts_with("dlv_"));
        assert_eq!(id.as_str().len(), 30);
    }

    #[test]
    fn add_is_idempotent_on_exact_id() {
        let mut d = Deliverable::new(WorkflowRunId::new("run_1"));
        let aid = ArtifactId::new("art_42");
        d.add(KindId::from("Issue"), aid.clone());
        d.add(KindId::from("Issue"), aid.clone());
        assert_eq!(d.get(&KindId::from("Issue")), &[aid]);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn add_accumulates_distinct_ids_per_kind() {
        let mut d = Deliverable::new(WorkflowRunId::new("run_1"));
        d.add(KindId::from("Commit"), ArtifactId::new("art_a"));
        d.add(KindId::from("Commit"), ArtifactId::new("art_b"));
        d.add(KindId::from("PR"), ArtifactId::new("art_pr"));
        assert_eq!(d.get(&KindId::from("Commit")).len(), 2);
        assert_eq!(d.get(&KindId::from("PR")).len(), 1);
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn deliverable_serde_roundtrip() {
        let mut d = Deliverable::new(WorkflowRunId::new("run_abc"));
        d.add(KindId::from("Issue"), ArtifactId::new("art_issue_1"));
        d.add(KindId::from("PR"), ArtifactId::new("art_pr_1"));

        let json = serde_json::to_string(&d).unwrap();
        let back: Deliverable = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn deliverable_empty_by_default() {
        let d = Deliverable::new(WorkflowRunId::new("run_1"));
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
    }
}
