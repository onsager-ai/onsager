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
}
