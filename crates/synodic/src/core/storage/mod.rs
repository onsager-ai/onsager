//! Persistent storage for governance data.
//!
//! Provides a `Storage` trait with PostgreSQL and SQLite implementations.
//! All governance state — rules, threat categories, feedback events, scores,
//! and probe results — lives in the database.
//!
//! - **PostgreSQL** for production (default in Docker)
//! - **SQLite** for local development and demos

pub mod pool;
#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "sqlite")]
pub mod sqlite;
#[cfg(test)]
mod tests;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A governance rule that can block agent tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub description: String,
    pub category_id: String,
    pub tools: Vec<String>,
    pub condition_type: String,
    pub condition_value: String,
    pub lifecycle: Lifecycle,
    pub alpha: i32,
    pub beta: i32,
    pub prior_alpha: i32,
    pub prior_beta: i32,
    pub enabled: bool,
    pub project_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub crystallized_at: Option<DateTime<Utc>>,
    pub cross_project_validated: bool,
}

/// Rule lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Candidate,
    Active,
    Tuned,
    Crystallized,
    Deprecated,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Tuned => "tuned",
            Self::Crystallized => "crystallized",
            Self::Deprecated => "deprecated",
        }
    }
}

impl std::str::FromStr for Lifecycle {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "candidate" => Ok(Self::Candidate),
            "active" => Ok(Self::Active),
            "tuned" => Ok(Self::Tuned),
            "crystallized" => Ok(Self::Crystallized),
            "deprecated" => Ok(Self::Deprecated),
            other => anyhow::bail!("unknown lifecycle state: {other}"),
        }
    }
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Fields that can be updated on a rule.
#[derive(Debug, Default)]
pub struct UpdateRule {
    pub description: Option<String>,
    pub condition_value: Option<String>,
    pub lifecycle: Option<Lifecycle>,
    pub enabled: Option<bool>,
    pub alpha_increment: Option<i32>,
    pub beta_increment: Option<i32>,
    pub cross_project_validated: Option<bool>,
    pub crystallized_at: Option<DateTime<Utc>>,
}

/// Parameters for creating a new rule.
#[derive(Debug, Clone)]
pub struct CreateRule {
    pub id: String,
    pub description: String,
    pub category_id: String,
    pub tools: Vec<String>,
    pub condition_type: String,
    pub condition_value: String,
    pub lifecycle: Lifecycle,
    pub prior_alpha: i32,
    pub prior_beta: i32,
    pub project_id: Option<String>,
}

/// A threat category from the governance threat taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatCategory {
    pub id: String,
    pub name: String,
    pub description: String,
    pub severity: String,
    pub severity_weight: f64,
    pub examples: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A feedback event (override, confirmed block, CI failure, incident).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub id: Uuid,
    pub signal_type: String,
    pub rule_id: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub override_reason: Option<String>,
    pub failure_type: Option<String>,
    pub evidence_url: Option<String>,
    pub project_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Filters for querying feedback events.
#[derive(Debug, Default)]
pub struct FeedbackFilters {
    pub rule_id: Option<String>,
    pub signal_type: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

/// A snapshot of governance scores at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceScores {
    pub id: Uuid,
    pub project_id: Option<String>,
    pub safety_score: f64,
    pub friction_score: f64,
    pub blocks_count: i32,
    pub override_count: i32,
    pub total_tool_calls: i32,
    pub coverage_score: f64,
    pub covered_categories: i32,
    pub total_categories: i32,
    pub converged: bool,
    pub rule_churn_rate: f64,
    pub created_at: DateTime<Utc>,
}

/// A pipeline run record for telemetry tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: String,
    pub prompt: String,
    pub branch: Option<String>,
    pub outcome: String,
    pub attempts: i32,
    pub model: Option<String>,
    pub build_duration_ms: Option<i64>,
    pub build_cost_usd: Option<f64>,
    pub inspect_duration_ms: Option<i64>,
    pub total_duration_ms: i64,
    pub project_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A governance event displayed on the web dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceEvent {
    pub id: String,
    pub event_type: String,
    pub title: String,
    pub severity: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub resolved: bool,
    pub resolution_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Parameters for creating a new governance event.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateGovernanceEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub title: String,
    pub severity: Option<String>,
    pub source: Option<String>,
}

/// Filters for querying governance events.
#[derive(Debug, Default)]
pub struct GovernanceEventFilters {
    pub event_type: Option<String>,
}

/// A rule-change proposal enqueued by Ising (issue #36 Step 2). Each row
/// corresponds to one `ising.rule_proposed` event the listener ingested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleProposal {
    pub id: String,
    /// Unique key from Ising — used by the listener for idempotent INSERTs.
    pub insight_id: String,
    pub signal_kind: String,
    pub subject_ref: String,
    /// Serialized `onsager_spine::factory_event::RuleProposalAction` — the
    /// Synodic listener stores it as JSON so new action kinds don't require
    /// a migration.
    pub proposed_action: serde_json::Value,
    pub class: String,
    pub rationale: String,
    pub confidence: f64,
    pub status: String,
    pub resolution_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Parameters for inserting a new rule proposal (source: Ising event).
#[derive(Debug, Clone)]
pub struct CreateRuleProposal {
    pub insight_id: String,
    pub signal_kind: String,
    pub subject_ref: String,
    pub proposed_action: serde_json::Value,
    pub class: String,
    pub rationale: String,
    pub confidence: f64,
    /// If `Some`, insert with this status (e.g. `"approved"` for safe-auto
    /// proposals the listener auto-applies). Defaults to `"pending"`.
    pub initial_status: Option<String>,
}

/// Result of an adversarial probe against a rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub id: Uuid,
    pub rule_id: String,
    pub strategy: String,
    pub probe_input: serde_json::Value,
    pub bypassed: bool,
    pub proposed_expansion: Option<String>,
    pub expansion_precision_drop: Option<f64>,
    pub expansion_approved: Option<bool>,
    pub created_at: DateTime<Utc>,
}

/// Cache key for the compiled `InterceptEngine` (issue #32).
///
/// Treat the value as an opaque token: the public API only supports
/// equality / hashing, not inspection of the component fields. The exact
/// encoding (currently `(COUNT(*), MAX(updated_at))`) is an implementation
/// detail and may change.
///
/// Contract: implementations MUST shift this token when a rule matching
/// `active_only` is created, updated, or deleted. Two equal revisions are
/// best-effort evidence that no such change happened — a backend with
/// coarse `updated_at` precision can in principle collide, so this is not
/// a linearizability guarantee. The current SQLite backend writes
/// millisecond-precision timestamps; Postgres writes microsecond TIMESTAMPTZ.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RulesRevision {
    count: i64,
    max_updated_at: String,
}

impl RulesRevision {
    pub fn new(count: i64, max_updated_at: impl Into<String>) -> Self {
        Self {
            count,
            max_updated_at: max_updated_at.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Storage trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Storage: Send + Sync {
    /// Run pending migrations.
    async fn migrate(&self) -> Result<()>;

    // -- Rules ---------------------------------------------------------------

    /// Get all rules, optionally filtered to active-only.
    async fn get_rules(&self, active_only: bool) -> Result<Vec<Rule>>;

    /// Cheap revision token for the rule set, used for engine caching
    /// (issue #32). Implementations MUST shift this token when a rule
    /// matching `active_only` is created, updated, or deleted. Equal
    /// revisions are best-effort evidence that no such change happened —
    /// backends with coarse `updated_at` precision may produce the same
    /// token for distinct states. See [`RulesRevision`] for the full
    /// contract.
    ///
    /// Implemented as a single SQL aggregate so the per-`/gate`-call cost
    /// is one round-trip instead of a full rule fetch.
    async fn get_rules_revision(&self, active_only: bool) -> Result<RulesRevision>;

    /// Get a single rule by ID.
    async fn get_rule(&self, id: &str) -> Result<Option<Rule>>;

    /// Create a new rule.
    async fn create_rule(&self, rule: CreateRule) -> Result<Rule>;

    /// Update an existing rule.
    async fn update_rule(&self, id: &str, update: UpdateRule) -> Result<()>;

    /// Delete a rule by ID.
    async fn delete_rule(&self, id: &str) -> Result<()>;

    // -- Threat taxonomy ----------------------------------------------------

    /// Get all threat categories.
    async fn get_threat_categories(&self) -> Result<Vec<ThreatCategory>>;

    /// Get a single threat category by ID.
    async fn get_threat_category(&self, id: &str) -> Result<Option<ThreatCategory>>;

    // -- Feedback events ----------------------------------------------------

    /// Record a feedback event (override, confirmed, ci_failure, incident).
    async fn record_feedback(&self, event: FeedbackEvent) -> Result<()>;

    /// Query feedback events with filters.
    async fn get_feedback(&self, filters: FeedbackFilters) -> Result<Vec<FeedbackEvent>>;

    // -- Scoring snapshots --------------------------------------------------

    /// Record a scoring snapshot.
    async fn record_scores(&self, scores: GovernanceScores) -> Result<()>;

    /// Get scoring snapshots for a project since a given time.
    async fn get_scores(
        &self,
        project_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<GovernanceScores>>;

    // -- Pipeline runs -------------------------------------------------------

    /// Record a pipeline run for telemetry.
    async fn record_pipeline_run(&self, run: PipelineRun) -> Result<()>;

    /// Get pipeline runs, optionally filtered by project.
    async fn get_pipeline_runs(
        &self,
        project_id: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<PipelineRun>>;

    // -- Probe results ------------------------------------------------------

    /// Record a probe result.
    async fn record_probe(&self, result: ProbeResult) -> Result<()>;

    /// Get probe results for a rule.
    async fn get_probes(&self, rule_id: &str) -> Result<Vec<ProbeResult>>;

    // -- Governance events (dashboard) --------------------------------------

    /// Get governance events, optionally filtered by type.
    async fn get_governance_events(
        &self,
        filters: GovernanceEventFilters,
    ) -> Result<Vec<GovernanceEvent>>;

    /// Get a single governance event by ID.
    async fn get_governance_event(&self, id: &str) -> Result<Option<GovernanceEvent>>;

    /// Create a new governance event.
    async fn create_governance_event(
        &self,
        event: CreateGovernanceEvent,
    ) -> Result<GovernanceEvent>;

    /// Resolve a governance event.
    async fn resolve_governance_event(&self, id: &str, notes: Option<String>) -> Result<()>;

    // -- Rule proposals (issue #36 Step 2) ----------------------------------

    /// List rule proposals, optionally filtered by status
    /// (`"pending"` / `"approved"` / `"rejected"`).
    async fn list_rule_proposals(&self, status: Option<&str>) -> Result<Vec<RuleProposal>>;

    /// Insert a new rule proposal. Deduplicates on `insight_id` — if a row
    /// with the same `insight_id` already exists the existing proposal is
    /// returned unchanged. The listener relies on this so redelivery
    /// (restart, catch-up) is a no-op.
    async fn create_rule_proposal(&self, proposal: CreateRuleProposal) -> Result<RuleProposal>;

    /// Transition a proposal to a terminal status (`"approved"` or
    /// `"rejected"`). Errors if the id doesn't exist or the proposal is
    /// already resolved.
    async fn resolve_rule_proposal(
        &self,
        id: &str,
        status: &str,
        notes: Option<String>,
    ) -> Result<()>;
}
