//! SQLite implementation of the Storage trait.
//!
//! Used for local development, demos, and single-user setups.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};

/// Format for timestamps written to rule columns. Uses millisecond precision
/// so `get_rules_revision` can distinguish updates landing within the same
/// second (issue #32 / PR #42 review): the cache keys off `MAX(updated_at)`
/// and second-precision collisions silently serve stale rules.
fn rule_ts_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use uuid::Uuid;

use super::*;

/// SQLite-backed storage.
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Connect to a SQLite database.
    ///
    /// The URL should be `sqlite://path/to/db.sqlite?mode=rwc`
    /// (`mode=rwc` creates the file if it doesn't exist).
    pub async fn connect(url: &str) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(path) = url.strip_prefix("sqlite://") {
            let path = path.split('?').next().unwrap_or(path);
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .context("opening SQLite database")?;

        // Enable WAL mode for better concurrent performance
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .ok();

        Ok(Self { pool })
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn migrate(&self) -> Result<()> {
        // Run schema migration
        let schema = include_str!("../../../migrations/001_initial_schema.sql");
        for statement in schema.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt)
                    .execute(&self.pool)
                    .await
                    .with_context(|| {
                        format!(
                            "migration statement failed: {}",
                            &stmt[..stmt.len().min(80)]
                        )
                    })?;
            }
        }

        // Run pipeline telemetry migration
        let telemetry = include_str!("../../../migrations/003_pipeline_runs.sql");
        for statement in telemetry.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt)
                    .execute(&self.pool)
                    .await
                    .with_context(|| {
                        format!(
                            "pipeline telemetry migration statement failed: {}",
                            &stmt[..stmt.len().min(80)]
                        )
                    })?;
            }
        }

        // Run governance events migration
        let gov_events = include_str!("../../../migrations/004_governance_events.sql");
        for statement in gov_events.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt)
                    .execute(&self.pool)
                    .await
                    .with_context(|| {
                        format!(
                            "governance events migration statement failed: {}",
                            &stmt[..stmt.len().min(80)]
                        )
                    })?;
            }
        }

        // Run rule proposals migration (issue #36 Step 2)
        let proposals = include_str!("../../../migrations/005_rule_proposals.sql");
        for statement in proposals.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt)
                    .execute(&self.pool)
                    .await
                    .with_context(|| {
                        format!(
                            "rule proposals migration statement failed: {}",
                            &stmt[..stmt.len().min(80)]
                        )
                    })?;
            }
        }

        // Run seed data
        let seed = include_str!("../../../migrations/002_seed_data.sql");
        for statement in seed.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt).execute(&self.pool).await.ok(); // OK to fail (INSERT OR IGNORE)
            }
        }

        Ok(())
    }

    // -- Rules ---------------------------------------------------------------

    async fn get_rules(&self, active_only: bool) -> Result<Vec<Rule>> {
        let rows = if active_only {
            sqlx::query_as::<_, RuleRow>("SELECT * FROM rules WHERE enabled = 1 ORDER BY id")
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as::<_, RuleRow>("SELECT * FROM rules ORDER BY id")
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter().map(|r| r.into_rule()).collect()
    }

    async fn get_rule(&self, id: &str) -> Result<Option<Rule>> {
        let row = sqlx::query_as::<_, RuleRow>("SELECT * FROM rules WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| r.into_rule()).transpose()
    }

    async fn get_rules_revision(&self, active_only: bool) -> Result<RulesRevision> {
        // SQLite stores `updated_at` as TEXT (RFC3339), so MAX(updated_at)
        // already returns a string-comparable token — no cast needed.
        let row: (i64, Option<String>) = if active_only {
            sqlx::query_as("SELECT COUNT(*), MAX(updated_at) FROM rules WHERE enabled = 1")
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_as("SELECT COUNT(*), MAX(updated_at) FROM rules")
                .fetch_one(&self.pool)
                .await?
        };
        Ok(RulesRevision::new(row.0, row.1.unwrap_or_default()))
    }

    async fn create_rule(&self, rule: CreateRule) -> Result<Rule> {
        let tools_json = serde_json::to_string(&rule.tools)?;
        let now = rule_ts_now();
        let lifecycle = rule.lifecycle.as_str();

        sqlx::query(
            "INSERT INTO rules (id, description, category_id, tools, condition_type, condition_value, lifecycle, alpha, beta, prior_alpha, prior_beta, enabled, project_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(&rule.id)
        .bind(&rule.description)
        .bind(&rule.category_id)
        .bind(&tools_json)
        .bind(&rule.condition_type)
        .bind(&rule.condition_value)
        .bind(lifecycle)
        .bind(rule.prior_alpha)
        .bind(rule.prior_beta)
        .bind(rule.prior_alpha)
        .bind(rule.prior_beta)
        .bind(&rule.project_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .context("inserting rule")?;

        self.get_rule(&rule.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("rule not found after insert"))
    }

    async fn update_rule(&self, id: &str, update: UpdateRule) -> Result<()> {
        let now = rule_ts_now();

        if let Some(desc) = &update.description {
            sqlx::query("UPDATE rules SET description = ?, updated_at = ? WHERE id = ?")
                .bind(desc)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(val) = &update.condition_value {
            sqlx::query("UPDATE rules SET condition_value = ?, updated_at = ? WHERE id = ?")
                .bind(val)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(lc) = &update.lifecycle {
            sqlx::query("UPDATE rules SET lifecycle = ?, updated_at = ? WHERE id = ?")
                .bind(lc.as_str())
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(en) = update.enabled {
            sqlx::query("UPDATE rules SET enabled = ?, updated_at = ? WHERE id = ?")
                .bind(en)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(inc) = update.alpha_increment {
            sqlx::query("UPDATE rules SET alpha = alpha + ?, updated_at = ? WHERE id = ?")
                .bind(inc)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(inc) = update.beta_increment {
            sqlx::query("UPDATE rules SET beta = beta + ?, updated_at = ? WHERE id = ?")
                .bind(inc)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(v) = update.cross_project_validated {
            sqlx::query(
                "UPDATE rules SET cross_project_validated = ?, updated_at = ? WHERE id = ?",
            )
            .bind(v)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        if let Some(ts) = &update.crystallized_at {
            let ts_str = ts.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            sqlx::query("UPDATE rules SET crystallized_at = ?, updated_at = ? WHERE id = ?")
                .bind(&ts_str)
                .bind(&now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn delete_rule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -- Threat taxonomy ----------------------------------------------------

    async fn get_threat_categories(&self) -> Result<Vec<ThreatCategory>> {
        let rows = sqlx::query_as::<_, CategoryRow>(
            "SELECT * FROM threat_categories ORDER BY severity_weight DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.into_category()).collect()
    }

    async fn get_threat_category(&self, id: &str) -> Result<Option<ThreatCategory>> {
        let row = sqlx::query_as::<_, CategoryRow>("SELECT * FROM threat_categories WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| r.into_category()).transpose()
    }

    // -- Feedback events ----------------------------------------------------

    async fn record_feedback(&self, event: FeedbackEvent) -> Result<()> {
        let id = event.id.to_string();
        let tool_input = serde_json::to_string(&event.tool_input)?;
        let created_at = event.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        sqlx::query(
            "INSERT INTO feedback_events (id, signal_type, rule_id, session_id, tool_name, tool_input, override_reason, failure_type, evidence_url, project_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&event.signal_type)
        .bind(&event.rule_id)
        .bind(&event.session_id)
        .bind(&event.tool_name)
        .bind(&tool_input)
        .bind(&event.override_reason)
        .bind(&event.failure_type)
        .bind(&event.evidence_url)
        .bind(&event.project_id)
        .bind(&created_at)
        .execute(&self.pool)
        .await
        .context("inserting feedback event")?;

        Ok(())
    }

    async fn get_feedback(&self, filters: FeedbackFilters) -> Result<Vec<FeedbackEvent>> {
        let mut sql = String::from("SELECT * FROM feedback_events WHERE 1=1");
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref rule_id) = filters.rule_id {
            sql.push_str(" AND rule_id = ?");
            binds.push(rule_id.clone());
        }
        if let Some(ref signal_type) = filters.signal_type {
            sql.push_str(" AND signal_type = ?");
            binds.push(signal_type.clone());
        }
        if let Some(ref session_id) = filters.session_id {
            sql.push_str(" AND session_id = ?");
            binds.push(session_id.clone());
        }
        if let Some(ref project_id) = filters.project_id {
            sql.push_str(" AND project_id = ?");
            binds.push(project_id.clone());
        }
        if let Some(ref since) = filters.since {
            sql.push_str(" AND created_at >= ?");
            binds.push(since.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filters.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut query = sqlx::query_as::<_, FeedbackRow>(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.into_event()).collect()
    }

    // -- Scoring snapshots --------------------------------------------------

    async fn record_scores(&self, scores: GovernanceScores) -> Result<()> {
        let id = scores.id.to_string();
        let created_at = scores.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        sqlx::query(
            "INSERT INTO scoring_snapshots (id, project_id, safety_score, friction_score, blocks_count, override_count, total_tool_calls, coverage_score, covered_categories, total_categories, converged, rule_churn_rate, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&scores.project_id)
        .bind(scores.safety_score)
        .bind(scores.friction_score)
        .bind(scores.blocks_count)
        .bind(scores.override_count)
        .bind(scores.total_tool_calls)
        .bind(scores.coverage_score)
        .bind(scores.covered_categories)
        .bind(scores.total_categories)
        .bind(scores.converged)
        .bind(scores.rule_churn_rate)
        .bind(&created_at)
        .execute(&self.pool)
        .await
        .context("inserting scoring snapshot")?;

        Ok(())
    }

    async fn get_scores(
        &self,
        project_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<GovernanceScores>> {
        let since_str = since.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let rows = if let Some(pid) = project_id {
            sqlx::query_as::<_, ScoresRow>(
                "SELECT * FROM scoring_snapshots WHERE project_id = ? AND created_at >= ? ORDER BY created_at",
            )
            .bind(pid)
            .bind(&since_str)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, ScoresRow>(
                "SELECT * FROM scoring_snapshots WHERE created_at >= ? ORDER BY created_at",
            )
            .bind(&since_str)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(|r| r.into_scores()).collect()
    }

    // -- Pipeline runs -------------------------------------------------------

    async fn record_pipeline_run(&self, run: PipelineRun) -> Result<()> {
        let created_at = run.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        sqlx::query(
            "INSERT INTO pipeline_runs (id, prompt, branch, outcome, attempts, model, build_duration_ms, build_cost_usd, inspect_duration_ms, total_duration_ms, project_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run.id)
        .bind(&run.prompt)
        .bind(&run.branch)
        .bind(&run.outcome)
        .bind(run.attempts)
        .bind(&run.model)
        .bind(run.build_duration_ms)
        .bind(run.build_cost_usd)
        .bind(run.inspect_duration_ms)
        .bind(run.total_duration_ms)
        .bind(&run.project_id)
        .bind(&created_at)
        .execute(&self.pool)
        .await
        .context("inserting pipeline run")?;

        Ok(())
    }

    async fn get_pipeline_runs(
        &self,
        project_id: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<PipelineRun>> {
        let limit_val = limit.unwrap_or(50);

        let rows = if let Some(pid) = project_id {
            sqlx::query_as::<_, PipelineRunRow>(
                "SELECT * FROM pipeline_runs WHERE project_id = ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(pid)
            .bind(limit_val)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PipelineRunRow>(
                "SELECT * FROM pipeline_runs ORDER BY created_at DESC LIMIT ?",
            )
            .bind(limit_val)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(|r| r.into_pipeline_run()).collect()
    }

    // -- Probe results ------------------------------------------------------

    async fn record_probe(&self, result: ProbeResult) -> Result<()> {
        let id = result.id.to_string();
        let probe_input = serde_json::to_string(&result.probe_input)?;
        let created_at = result.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        sqlx::query(
            "INSERT INTO probe_results (id, rule_id, strategy, probe_input, bypassed, proposed_expansion, expansion_precision_drop, expansion_approved, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&result.rule_id)
        .bind(&result.strategy)
        .bind(&probe_input)
        .bind(result.bypassed)
        .bind(&result.proposed_expansion)
        .bind(result.expansion_precision_drop)
        .bind(result.expansion_approved)
        .bind(&created_at)
        .execute(&self.pool)
        .await
        .context("inserting probe result")?;

        Ok(())
    }

    async fn get_probes(&self, rule_id: &str) -> Result<Vec<ProbeResult>> {
        let rows = sqlx::query_as::<_, ProbeRow>(
            "SELECT * FROM probe_results WHERE rule_id = ? ORDER BY created_at DESC",
        )
        .bind(rule_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.into_probe()).collect()
    }

    // -- Governance events (dashboard) --------------------------------------

    async fn get_governance_events(
        &self,
        filters: GovernanceEventFilters,
    ) -> Result<Vec<GovernanceEvent>> {
        let rows = if let Some(ref event_type) = filters.event_type {
            sqlx::query_as::<_, GovEventRow>(
                "SELECT * FROM governance_events WHERE event_type = ? ORDER BY created_at DESC",
            )
            .bind(event_type)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, GovEventRow>(
                "SELECT * FROM governance_events ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(|r| r.into_event()).collect()
    }

    async fn get_governance_event(&self, id: &str) -> Result<Option<GovernanceEvent>> {
        let row = sqlx::query_as::<_, GovEventRow>("SELECT * FROM governance_events WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| r.into_event()).transpose()
    }

    async fn create_governance_event(
        &self,
        event: CreateGovernanceEvent,
    ) -> Result<GovernanceEvent> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let severity = event.severity.unwrap_or_else(|| "medium".to_string());
        let source = event.source.unwrap_or_else(|| "api".to_string());

        sqlx::query(
            "INSERT INTO governance_events (id, event_type, title, severity, source, metadata, resolved, created_at)
             VALUES (?, ?, ?, ?, ?, '{}', 0, ?)",
        )
        .bind(&id)
        .bind(&event.event_type)
        .bind(&event.title)
        .bind(&severity)
        .bind(&source)
        .bind(&now)
        .execute(&self.pool)
        .await
        .context("inserting governance event")?;

        self.get_governance_event(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("governance event not found after insert"))
    }

    async fn resolve_governance_event(&self, id: &str, notes: Option<String>) -> Result<()> {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        sqlx::query(
            "UPDATE governance_events SET resolved = 1, resolution_notes = ?, resolved_at = ? WHERE id = ?",
        )
        .bind(&notes)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // -- Rule proposals (issue #36 Step 2) ----------------------------------

    async fn list_rule_proposals(&self, status: Option<&str>) -> Result<Vec<RuleProposal>> {
        let rows = if let Some(s) = status {
            sqlx::query_as::<_, ProposalRow>(
                "SELECT * FROM rule_proposals WHERE status = ? ORDER BY created_at DESC",
            )
            .bind(s)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, ProposalRow>(
                "SELECT * FROM rule_proposals ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(|r| r.into_proposal()).collect()
    }

    async fn create_rule_proposal(&self, proposal: CreateRuleProposal) -> Result<RuleProposal> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let status = proposal
            .initial_status
            .as_deref()
            .unwrap_or("pending")
            .to_string();
        let action_json = serde_json::to_string(&proposal.proposed_action)?;

        // ON CONFLICT keeps the first proposal per insight — redelivery is a
        // no-op. SQLite returns rowcount 0 when the conflict path fires;
        // either way, the SELECT below resolves to the current row.
        sqlx::query(
            "INSERT INTO rule_proposals (id, insight_id, signal_kind, subject_ref, \
             proposed_action, class, rationale, confidence, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(insight_id) DO NOTHING",
        )
        .bind(&id)
        .bind(&proposal.insight_id)
        .bind(&proposal.signal_kind)
        .bind(&proposal.subject_ref)
        .bind(&action_json)
        .bind(&proposal.class)
        .bind(&proposal.rationale)
        .bind(proposal.confidence)
        .bind(&status)
        .bind(&now)
        .execute(&self.pool)
        .await
        .context("inserting rule proposal")?;

        let row =
            sqlx::query_as::<_, ProposalRow>("SELECT * FROM rule_proposals WHERE insight_id = ?")
                .bind(&proposal.insight_id)
                .fetch_one(&self.pool)
                .await
                .context("reading back rule proposal")?;
        row.into_proposal()
    }

    async fn resolve_rule_proposal(
        &self,
        id: &str,
        status: &str,
        notes: Option<String>,
    ) -> Result<()> {
        if status != "approved" && status != "rejected" {
            anyhow::bail!("invalid proposal status: {status}");
        }
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        // Guard against double-resolution: only transition rows still
        // pending, so a repeated PATCH can't overwrite the original
        // resolver's notes.
        let result = sqlx::query(
            "UPDATE rule_proposals SET status = ?, resolution_notes = ?, resolved_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(&notes)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            anyhow::bail!("proposal not found or already resolved: {id}");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row types — map DB rows to domain types
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: String,
    description: String,
    category_id: String,
    tools: String,
    condition_type: String,
    condition_value: String,
    lifecycle: String,
    alpha: i32,
    beta: i32,
    prior_alpha: i32,
    prior_beta: i32,
    enabled: bool,
    project_id: Option<String>,
    created_at: String,
    updated_at: String,
    crystallized_at: Option<String>,
    cross_project_validated: bool,
}

impl RuleRow {
    fn into_rule(self) -> Result<Rule> {
        Ok(Rule {
            id: self.id,
            description: self.description,
            category_id: self.category_id,
            tools: serde_json::from_str(&self.tools).unwrap_or_default(),
            condition_type: self.condition_type,
            condition_value: self.condition_value,
            lifecycle: self.lifecycle.parse()?,
            alpha: self.alpha,
            beta: self.beta,
            prior_alpha: self.prior_alpha,
            prior_beta: self.prior_beta,
            enabled: self.enabled,
            project_id: self.project_id,
            created_at: parse_datetime(&self.created_at)?,
            updated_at: parse_datetime(&self.updated_at)?,
            crystallized_at: self
                .crystallized_at
                .as_deref()
                .map(parse_datetime)
                .transpose()?,
            cross_project_validated: self.cross_project_validated,
        })
    }
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: String,
    name: String,
    description: String,
    severity: String,
    severity_weight: f64,
    examples: String,
    created_at: String,
    updated_at: String,
}

impl CategoryRow {
    fn into_category(self) -> Result<ThreatCategory> {
        Ok(ThreatCategory {
            id: self.id,
            name: self.name,
            description: self.description,
            severity: self.severity,
            severity_weight: self.severity_weight,
            examples: serde_json::from_str(&self.examples).unwrap_or_default(),
            created_at: parse_datetime(&self.created_at)?,
            updated_at: parse_datetime(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct FeedbackRow {
    id: String,
    signal_type: String,
    rule_id: String,
    session_id: Option<String>,
    tool_name: String,
    tool_input: String,
    override_reason: Option<String>,
    failure_type: Option<String>,
    evidence_url: Option<String>,
    project_id: Option<String>,
    created_at: String,
}

impl FeedbackRow {
    fn into_event(self) -> Result<FeedbackEvent> {
        Ok(FeedbackEvent {
            id: Uuid::parse_str(&self.id)?,
            signal_type: self.signal_type,
            rule_id: self.rule_id,
            session_id: self.session_id,
            tool_name: self.tool_name,
            tool_input: serde_json::from_str(&self.tool_input).unwrap_or_default(),
            override_reason: self.override_reason,
            failure_type: self.failure_type,
            evidence_url: self.evidence_url,
            project_id: self.project_id,
            created_at: parse_datetime(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ScoresRow {
    id: String,
    project_id: Option<String>,
    safety_score: f64,
    friction_score: f64,
    blocks_count: i32,
    override_count: i32,
    total_tool_calls: i32,
    coverage_score: f64,
    covered_categories: i32,
    total_categories: i32,
    converged: bool,
    rule_churn_rate: f64,
    created_at: String,
}

impl ScoresRow {
    fn into_scores(self) -> Result<GovernanceScores> {
        Ok(GovernanceScores {
            id: Uuid::parse_str(&self.id)?,
            project_id: self.project_id,
            safety_score: self.safety_score,
            friction_score: self.friction_score,
            blocks_count: self.blocks_count,
            override_count: self.override_count,
            total_tool_calls: self.total_tool_calls,
            coverage_score: self.coverage_score,
            covered_categories: self.covered_categories,
            total_categories: self.total_categories,
            converged: self.converged,
            rule_churn_rate: self.rule_churn_rate,
            created_at: parse_datetime(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ProbeRow {
    id: String,
    rule_id: String,
    strategy: String,
    probe_input: String,
    bypassed: bool,
    proposed_expansion: Option<String>,
    expansion_precision_drop: Option<f64>,
    expansion_approved: Option<bool>,
    created_at: String,
}

impl ProbeRow {
    fn into_probe(self) -> Result<ProbeResult> {
        Ok(ProbeResult {
            id: Uuid::parse_str(&self.id)?,
            rule_id: self.rule_id,
            strategy: self.strategy,
            probe_input: serde_json::from_str(&self.probe_input).unwrap_or_default(),
            bypassed: self.bypassed,
            proposed_expansion: self.proposed_expansion,
            expansion_precision_drop: self.expansion_precision_drop,
            expansion_approved: self.expansion_approved,
            created_at: parse_datetime(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PipelineRunRow {
    id: String,
    prompt: String,
    branch: Option<String>,
    outcome: String,
    attempts: i32,
    model: Option<String>,
    build_duration_ms: Option<i64>,
    build_cost_usd: Option<f64>,
    inspect_duration_ms: Option<i64>,
    total_duration_ms: i64,
    project_id: Option<String>,
    created_at: String,
}

impl PipelineRunRow {
    fn into_pipeline_run(self) -> Result<PipelineRun> {
        Ok(PipelineRun {
            id: self.id,
            prompt: self.prompt,
            branch: self.branch,
            outcome: self.outcome,
            attempts: self.attempts,
            model: self.model,
            build_duration_ms: self.build_duration_ms,
            build_cost_usd: self.build_cost_usd,
            inspect_duration_ms: self.inspect_duration_ms,
            total_duration_ms: self.total_duration_ms,
            project_id: self.project_id,
            created_at: parse_datetime(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct GovEventRow {
    id: String,
    event_type: String,
    title: String,
    severity: String,
    source: String,
    metadata: String,
    resolved: bool,
    resolution_notes: Option<String>,
    created_at: String,
    resolved_at: Option<String>,
}

impl GovEventRow {
    fn into_event(self) -> Result<GovernanceEvent> {
        Ok(GovernanceEvent {
            id: self.id,
            event_type: self.event_type,
            title: self.title,
            severity: self.severity,
            source: self.source,
            metadata: serde_json::from_str(&self.metadata).unwrap_or_default(),
            resolved: self.resolved,
            resolution_notes: self.resolution_notes,
            created_at: parse_datetime(&self.created_at)?,
            resolved_at: self
                .resolved_at
                .as_deref()
                .map(parse_datetime)
                .transpose()?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ProposalRow {
    id: String,
    insight_id: String,
    signal_kind: String,
    subject_ref: String,
    proposed_action: String,
    class: String,
    rationale: String,
    confidence: f64,
    status: String,
    resolution_notes: Option<String>,
    created_at: String,
    resolved_at: Option<String>,
}

impl ProposalRow {
    fn into_proposal(self) -> Result<RuleProposal> {
        Ok(RuleProposal {
            id: self.id,
            insight_id: self.insight_id,
            signal_kind: self.signal_kind,
            subject_ref: self.subject_ref,
            proposed_action: serde_json::from_str(&self.proposed_action)
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
            class: self.class,
            rationale: self.rationale,
            confidence: self.confidence,
            status: self.status,
            resolution_notes: self.resolution_notes,
            created_at: parse_datetime(&self.created_at)?,
            resolved_at: self
                .resolved_at
                .as_deref()
                .map(parse_datetime)
                .transpose()?,
        })
    }
}

fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    // Try RFC3339 first — this handles both second-precision `...HH:MM:SSZ`
    // and millisecond-precision `...HH:MM:SS.fffZ` forms written by
    // different code paths in this file, plus any other offset variant that
    // ends up in the TEXT column.
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").map(|dt| dt.and_utc())
        })
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(|dt| dt.and_utc())
        })
        .with_context(|| format!("parsing datetime: {s}"))
}
