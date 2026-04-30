//! Artifact data model — the core value object of the Onsager factory.
//!
//! See `specs/artifact-model-v0.1.md` for the full specification. This module
//! provides the Rust types that all four subsystems share when talking about
//! artifacts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Globally unique, stable artifact identifier.
///
/// Format: `art_<26-char-ulid>`, e.g. `art_01HXYZABC123DEFGHJKMNPQRST`.
/// ULIDs are 128-bit time-sortable identifiers: lexicographic order matches
/// creation order, and collision probability is negligible at any realistic scale.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Create from a raw string. Caller is responsible for uniqueness.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new unique artifact ID using ULID.
    ///
    /// Returns a string of the form `art_<26-char-ulid>`.
    pub fn generate() -> Self {
        let ulid = ulid::Ulid::new().to_string();
        Self(format!("art_{ulid}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Artifact version identity
// ---------------------------------------------------------------------------

/// Content-addressed identifier for an artifact version snapshot.
///
/// Format: `ver_<64-char-hex>`, where the hex is the SHA-256 of
/// `(artifact_id, version, canonical_manifest_bytes)`. Two seals of the same
/// artifact at the same version with identical file contents produce the same
/// id (idempotent reseal → `VersionConflict`). Two different artifacts with
/// identical files produce different ids, so they do not collide.
///
/// The legacy `bnd_` prefix is accepted on read for one release cycle; new
/// ids are minted with `ver_`. The type was previously named `BundleId` (PR
/// #107); the alias was retired in #149 and call sites now reference
/// `ArtifactVersionId` directly.
///
/// This type lives here (not in `onsager-warehouse`) so that `Artifact` can
/// reference versions without creating an artifact↔warehouse cycle. The
/// hashing logic that derives an `ArtifactVersionId` from a manifest lives
/// in `onsager-warehouse`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactVersionId(String);

impl ArtifactVersionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactVersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Kind
// ---------------------------------------------------------------------------

/// Artifact type tag. Typed but not closed — users may register custom kinds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Code,
    Document,
    PullRequest,
    GithubIssue,
    /// User-defined kind not in the built-in set.
    Custom(String),
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Code => write!(f, "code"),
            Kind::Document => write!(f, "document"),
            Kind::PullRequest => write!(f, "pull_request"),
            Kind::GithubIssue => write!(f, "github_issue"),
            Kind::Custom(s) => write!(f, "{s}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle state
// ---------------------------------------------------------------------------

/// Artifact lifecycle state machine.
///
/// ```text
/// draft -> in_progress -> under_review -> released -> archived
///                ^                           |
///                +---------- revise ---------+
/// ```
///
/// Any state may transition directly to `archived` (early termination).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Draft,
    InProgress,
    UnderReview,
    Released,
    Archived,
}

impl ArtifactState {
    /// Check whether a transition from `self` to `target` is valid.
    pub fn can_transition_to(self, target: ArtifactState) -> bool {
        use ArtifactState::*;
        // Any state -> Archived is always valid (early termination).
        if target == Archived {
            return true;
        }
        matches!(
            (self, target),
            (Draft, InProgress)
                | (InProgress, UnderReview)
                | (UnderReview, Released)
                | (Released, InProgress) // revise cycle
        )
    }
}

impl fmt::Display for ArtifactState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ArtifactState::Draft => "draft",
            ArtifactState::InProgress => "in_progress",
            ArtifactState::UnderReview => "under_review",
            ArtifactState::Released => "released",
            ArtifactState::Archived => "archived",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// A concrete snapshot in an artifact's lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactVersion {
    /// Monotonically increasing version number. Never reused.
    pub version: u32,
    pub created_at: DateTime<Utc>,
    /// The session that shaped this version.
    pub created_by_session: String,
    /// Pointer to external storage (URI + optional checksum).
    pub content_ref: ContentRef,
    /// Semantic summary of what changed from the previous version.
    pub change_summary: String,
    /// Usually `version - 1`. Branching reserved for future use.
    pub parent_version: Option<u32>,
}

/// URI + optional checksum pointing to external content storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentRef {
    /// URI to the content (git ref, S3 key, Notion page, etc.).
    pub uri: String,
    /// Optional content checksum for integrity verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// Git hosting context for pull request-oriented artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitContext {
    pub repo: String,
    pub base_branch: String,
    pub head_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Lineage
// ---------------------------------------------------------------------------

/// Vertical lineage entry — which session shaped which version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerticalLineage {
    pub session_id: String,
    pub version: u32,
    pub recorded_at: DateTime<Utc>,
}

/// Horizontal lineage entry — which other artifact was used as input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizontalLineage {
    /// The artifact that was used as input.
    pub source_artifact_id: ArtifactId,
    /// The version of the source artifact that was referenced.
    pub source_version: u32,
    /// The role this input played (e.g., "reference", "template", "dependency").
    pub role: String,
    pub recorded_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Quality signals
// ---------------------------------------------------------------------------

/// A single append-only record about artifact quality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualitySignal {
    /// Signal origin.
    pub source: QualitySource,
    /// Dimension name (correctness, completeness, safety, readability, ...).
    pub dimension: String,
    /// Scalar score or discrete label.
    pub value: QualityValue,
    pub recorded_at: DateTime<Utc>,
    /// The entity that produced this signal.
    pub recorded_by: String,
}

/// Where a quality signal came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualitySource {
    AutomatedTest,
    Lint,
    HumanReview,
    IsingInference,
    SynodicCheck,
    External,
}

/// Quality signal value — either a numeric score or a discrete label.
///
/// `f64` blocks `Eq`; only `PartialEq` is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QualityValue {
    Score(f64),
    Label(String),
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

/// A declared consumer of an artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consumer {
    /// Consumer identifier (user ID, team ID, system name, endpoint URL).
    pub id: String,
    /// Consumer type for routing.
    #[serde(rename = "type")]
    pub consumer_type: ConsumerType,
}

/// The kind of consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerType {
    User,
    Team,
    System,
    ExternalEndpoint,
}

// ---------------------------------------------------------------------------
// The artifact itself
// ---------------------------------------------------------------------------

/// The top-level artifact entity.
///
/// This is the metadata record that Onsager holds. Content lives externally,
/// pointed to by `content_ref` in each version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: ArtifactId,
    pub kind: Kind,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_context: Option<GitContext>,

    // Ownership
    pub owner: String,
    pub consumers: Vec<Consumer>,

    // State
    pub state: ArtifactState,
    pub current_version: u32,

    // Warehouse pointer (warehouse-and-delivery-v0.1 §4.1).
    //
    // `current_version_id` advances on each successful release; a rework does
    // not clear it until the new version is sealed. `version_history` is
    // append-only and records every sealed version in order. Serde accepts
    // the legacy `current_bundle_id` / `bundle_history` keys one release cycle
    // (issue #101).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "current_bundle_id"
    )]
    pub current_version_id: Option<ArtifactVersionId>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "bundle_history"
    )]
    pub version_history: Vec<ArtifactVersionId>,

    // History (loaded on demand in practice, but modeled here for completeness)
    pub versions: Vec<ArtifactVersion>,
    pub vertical_lineage: Vec<VerticalLineage>,
    pub horizontal_lineage: Vec<HorizontalLineage>,
    pub quality_signals: Vec<QualitySignal>,

    // Workflow runtime tagging (issue #80).
    //
    // `workflow_id` attaches the artifact to a declared workflow's stage
    // chain. `current_stage_index` is the 0-based stage the artifact is
    // currently at; `None` means "no longer workflow-managed" (either
    // unregistered or past the last stage). `workflow_parked_reason`
    // carries the first gate-failure message when the stage runner parks
    // an artifact in UnderReview waiting for the failing gate to recover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_stage_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_parked_reason: Option<String>,
}

impl Artifact {
    /// Record a newly sealed artifact version: advance `current_version_id`
    /// and append to `version_history` (warehouse-and-delivery-v0.1 §6.3,
    /// invariant §9.4).
    pub fn record_version(&mut self, version_id: ArtifactVersionId) {
        self.version_history.push(version_id.clone());
        self.current_version_id = Some(version_id);
    }

    /// Record an `Issue → PR` referential link on this artifact (issue #103).
    ///
    /// Appends a `HorizontalLineage` entry with `role = "closes_issue"` so
    /// the agent-session → PR flow populates the Deliverable's cross-kind
    /// lineage without inventing a new lineage type. `source_version` is
    /// the issue version at the moment the PR was created; if the caller
    /// doesn't track an issue version, `0` is a conventional placeholder.
    pub fn record_closes_issue(&mut self, issue_id: ArtifactId, source_version: u32) {
        self.horizontal_lineage.push(HorizontalLineage {
            source_artifact_id: issue_id,
            source_version,
            role: "closes_issue".into(),
            recorded_at: Utc::now(),
        });
    }

    /// Create a new artifact in `Draft` state with no versions yet.
    pub fn new(
        kind: Kind,
        name: impl Into<String>,
        owner: impl Into<String>,
        created_by: impl Into<String>,
        consumers: Vec<Consumer>,
    ) -> Self {
        Self {
            artifact_id: ArtifactId::generate(),
            kind,
            name: name.into(),
            created_at: Utc::now(),
            created_by: created_by.into(),
            git_context: None,
            owner: owner.into(),
            consumers,
            state: ArtifactState::Draft,
            current_version: 0,
            current_version_id: None,
            version_history: Vec::new(),
            versions: Vec::new(),
            vertical_lineage: Vec::new(),
            horizontal_lineage: Vec::new(),
            quality_signals: Vec::new(),
            workflow_id: None,
            current_stage_index: None,
            workflow_parked_reason: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_id_format() {
        let id = ArtifactId::generate();
        assert!(id.as_str().starts_with("art_"));
        assert_eq!(id.as_str().len(), 30); // "art_" + 26-char ULID
    }

    #[test]
    fn state_transitions_valid() {
        use ArtifactState::*;
        assert!(Draft.can_transition_to(InProgress));
        assert!(InProgress.can_transition_to(UnderReview));
        assert!(UnderReview.can_transition_to(Released));
        assert!(Released.can_transition_to(InProgress)); // revise
    }

    #[test]
    fn state_transitions_invalid() {
        use ArtifactState::*;
        assert!(!Draft.can_transition_to(Released));
        assert!(!Draft.can_transition_to(UnderReview));
        assert!(!InProgress.can_transition_to(Draft));
        assert!(!Released.can_transition_to(Draft));
        assert!(!Released.can_transition_to(UnderReview));
    }

    #[test]
    fn any_state_can_archive() {
        use ArtifactState::*;
        for state in [Draft, InProgress, UnderReview, Released] {
            assert!(state.can_transition_to(Archived));
        }
    }

    #[test]
    fn archived_is_terminal() {
        use ArtifactState::*;
        for target in [Draft, InProgress, UnderReview, Released] {
            assert!(!Archived.can_transition_to(target));
        }
    }

    #[test]
    fn kind_serialization() {
        let kind = Kind::Code;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, r#""code""#);

        let pr = Kind::PullRequest;
        let json = serde_json::to_string(&pr).unwrap();
        assert_eq!(json, r#""pull_request""#);

        let custom = Kind::Custom("test_execution_report".into());
        let json = serde_json::to_string(&custom).unwrap();
        assert!(json.contains("test_execution_report"));
    }

    #[test]
    fn new_artifact_defaults() {
        let art = Artifact::new(
            Kind::PullRequest,
            "Q1 Feature Work",
            "marvin",
            "system",
            vec![Consumer {
                id: "analytics-team".into(),
                consumer_type: ConsumerType::Team,
            }],
        );
        assert_eq!(art.state, ArtifactState::Draft);
        assert_eq!(art.current_version, 0);
        assert!(art.git_context.is_none());
        assert!(art.versions.is_empty());
        assert!(!art.consumers.is_empty());
        assert!(art.current_version_id.is_none());
        assert!(art.version_history.is_empty());
    }

    #[test]
    fn record_version_advances_current_and_history() {
        let mut art = Artifact::new(Kind::Code, "svc", "marvin", "system", vec![]);
        let v1 = ArtifactVersionId::new("ver_v1");
        let v2 = ArtifactVersionId::new("ver_v2");

        art.record_version(v1.clone());
        assert_eq!(art.current_version_id.as_ref(), Some(&v1));
        assert_eq!(art.version_history, vec![v1.clone()]);

        art.record_version(v2.clone());
        // Current pointer advances to newest; history is append-only.
        assert_eq!(art.current_version_id.as_ref(), Some(&v2));
        assert_eq!(art.version_history, vec![v1, v2]);
    }

    #[test]
    fn record_closes_issue_appends_horizontal_lineage_with_role() {
        let mut pr = Artifact::new(Kind::PullRequest, "feature pr", "marvin", "session", vec![]);
        let issue_id = ArtifactId::new("art_issue_42");
        pr.record_closes_issue(issue_id.clone(), 1);

        assert_eq!(pr.horizontal_lineage.len(), 1);
        let link = &pr.horizontal_lineage[0];
        assert_eq!(link.source_artifact_id, issue_id);
        assert_eq!(link.role, "closes_issue");
        assert_eq!(link.source_version, 1);
    }

    #[test]
    fn artifact_version_id_serde_accepts_legacy_prefix() {
        let legacy = ArtifactVersionId::new("bnd_legacy");
        let json = serde_json::to_string(&legacy).unwrap();
        let roundtrip: ArtifactVersionId = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, legacy);

        let modern = ArtifactVersionId::new("ver_modern");
        let json = serde_json::to_string(&modern).unwrap();
        assert_eq!(json, r#""ver_modern""#);
    }

    #[test]
    fn artifact_serde_accepts_legacy_bundle_field_names() {
        let json = serde_json::json!({
            "artifact_id": "art_01HXYZABC123DEFGHJKMNPQRST",
            "kind": "code",
            "name": "svc",
            "created_at": "2026-04-22T00:00:00Z",
            "created_by": "system",
            "owner": "marvin",
            "consumers": [],
            "state": "draft",
            "current_version": 0,
            "current_bundle_id": "bnd_legacy",
            "bundle_history": ["bnd_legacy"],
            "versions": [],
            "vertical_lineage": [],
            "horizontal_lineage": [],
            "quality_signals": []
        });
        let art: Artifact = serde_json::from_value(json).unwrap();
        assert_eq!(
            art.current_version_id,
            Some(ArtifactVersionId::new("bnd_legacy"))
        );
        assert_eq!(
            art.version_history,
            vec![ArtifactVersionId::new("bnd_legacy")]
        );
    }

    #[test]
    fn git_context_serde_roundtrip() {
        let context = GitContext {
            repo: "onsager-ai/onsager".into(),
            base_branch: "main".into(),
            head_branch: "copilot/feature-pr-kind".into(),
            pr_number: Some(42),
            pr_url: Some("https://github.com/onsager-ai/onsager/pull/42".into()),
        };

        let json = serde_json::to_value(&context).unwrap();
        let roundtrip: GitContext = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, context);
    }

    #[test]
    fn custom_report_escape_hatch() {
        let kind = Kind::Custom("report".into());
        let json = serde_json::to_string(&kind).unwrap();
        let roundtrip: Kind = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, kind);
    }

    #[test]
    fn quality_value_serde() {
        let score = QualityValue::Score(0.95);
        let json = serde_json::to_string(&score).unwrap();
        assert_eq!(json, "0.95");

        let label = QualityValue::Label("pass".into());
        let json = serde_json::to_string(&label).unwrap();
        assert_eq!(json, r#""pass""#);
    }

    #[test]
    fn artifact_id_generate_uniqueness() {
        use std::collections::HashSet;
        let ids: HashSet<String> = (0..10_000)
            .map(|_| ArtifactId::generate().as_str().to_owned())
            .collect();
        assert_eq!(
            ids.len(),
            10_000,
            "expected 10 000 unique IDs, got duplicates"
        );
    }

    #[test]
    fn artifact_id_generate_lexicographic_order() {
        // ULID is time-sortable: IDs generated in an earlier millisecond
        // always sort before IDs generated in a later millisecond.
        // Two batches separated by a 2ms sleep land in different milliseconds,
        // so every ID in batch_b must sort after every ID in batch_a.
        let batch_a: Vec<String> = (0..5)
            .map(|_| ArtifactId::generate().as_str().to_owned())
            .collect();

        std::thread::sleep(std::time::Duration::from_millis(2));

        let batch_b: Vec<String> = (0..5)
            .map(|_| ArtifactId::generate().as_str().to_owned())
            .collect();

        let max_a = batch_a.iter().max().unwrap();
        let min_b = batch_b.iter().min().unwrap();
        assert!(
            min_b > max_a,
            "later batch should sort after earlier batch: max_a={max_a}, min_b={min_b}"
        );
    }
}
