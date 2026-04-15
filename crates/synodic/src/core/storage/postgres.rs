//! PostgreSQL implementation of the Storage trait.
//!
//! Used for production deployments and multi-user setups.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use super::*;

/// PostgreSQL-backed storage.
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Connect to a PostgreSQL database.
    ///
    /// The URL should be `postgres://user:pass@host/dbname`.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(url)
            .await
            .context("opening PostgreSQL database")?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn migrate(&self) -> Result<()> {
        // Run schema migration
        let schema = include_str!("../../../migrations/pg/001_initial_schema.sql");
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
        let telemetry = include_str!("../../../migrations/pg/003_pipeline_runs.sql");
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
        let gov_events = include_str!("../../../migrations/pg/004_governance_events.sql");
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

        // Run seed data
        let seed = include_str!("../../../migrations/pg/002_seed_data.sql");
        for statement in seed.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt)
                    .execute(&self.pool)
                    .await
                    .with_context(|| {
                        format!(
                            "seed data statement failed: {}",
                            &stmt[..stmt.len().min(80)]
                        )
                    })?;
            }
        }

        Ok(())
    }

    // -- Rules ---------------------------------------------------------------

    async fn get_rules(&self, active_only: bool) -> Result<Vec<Rule>> {
        let rows = if active_only {
            sqlx::query_as::<_, PgRuleRow>("SELECT * FROM rules WHERE enabled = TRUE ORDER BY id")
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query_as::<_, PgRuleRow>("SELECT * FROM rules ORDER BY id")
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter().map(|r| r.into_rule()).collect()
    }

    async fn get_rule(&self, id: &str) -> Result<Option<Rule>> {
        let row = sqlx::query_as::<_, PgRuleRow>("SELECT * FROM rules WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| r.into_rule()).transpose()
    }

    async fn create_rule(&self, rule: CreateRule) -> Result<Rule> {
        let tools_json = serde_json::to_value(&rule.tools)?;
        let now = Utc::now();
        let lifecycle = rule.lifecycle.as_str();

        sqlx::query(
            "INSERT INTO rules (id, description, category_id, tools, condition_type, condition_value, lifecycle, alpha, beta, prior_alpha, prior_beta, enabled, project_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, TRUE, $12, $13, $14)",
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
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("inserting rule")?;

        self.get_rule(&rule.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("rule not found after insert"))
    }

    async fn update_rule(&self, id: &str, update: UpdateRule) -> Result<()> {
        let now = Utc::now();

        if let Some(desc) = &update.description {
            sqlx::query("UPDATE rules SET description = $1, updated_at = $2 WHERE id = $3")
                .bind(desc)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(val) = &update.condition_value {
            sqlx::query("UPDATE rules SET condition_value = $1, updated_at = $2 WHERE id = $3")
                .bind(val)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(lc) = &update.lifecycle {
            sqlx::query("UPDATE rules SET lifecycle = $1, updated_at = $2 WHERE id = $3")
                .bind(lc.as_str())
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(en) = update.enabled {
            sqlx::query("UPDATE rules SET enabled = $1, updated_at = $2 WHERE id = $3")
                .bind(en)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(inc) = update.alpha_increment {
            sqlx::query("UPDATE rules SET alpha = alpha + $1, updated_at = $2 WHERE id = $3")
                .bind(inc)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(inc) = update.beta_increment {
            sqlx::query("UPDATE rules SET beta = beta + $1, updated_at = $2 WHERE id = $3")
                .bind(inc)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        if let Some(v) = update.cross_project_validated {
            sqlx::query(
                "UPDATE rules SET cross_project_validated = $1, updated_at = $2 WHERE id = $3",
            )
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        if let Some(ts) = &update.crystallized_at {
            sqlx::query("UPDATE rules SET crystallized_at = $1, updated_at = $2 WHERE id = $3")
                .bind(ts)
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn delete_rule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM rules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -- Threat taxonomy ----------------------------------------------------

    async fn get_threat_categories(&self) -> Result<Vec<ThreatCategory>> {
        let rows = sqlx::query_as::<_, PgCategoryRow>(
            "SELECT * FROM threat_categories ORDER BY severity_weight DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|r| r.into_category()).collect()
    }

    async fn get_threat_category(&self, id: &str) -> Result<Option<ThreatCategory>> {
        let row =
            sqlx::query_as::<_, PgCategoryRow>("SELECT * FROM threat_categories WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        row.map(|r| r.into_category()).transpose()
    }

    // -- Feedback events ----------------------------------------------------

    async fn record_feedback(&self, event: FeedbackEvent) -> Result<()> {
        let id = event.id.to_string();

        sqlx::query(
            "INSERT INTO feedback_events (id, signal_type, rule_id, session_id, tool_name, tool_input, override_reason, failure_type, evidence_url, project_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&id)
        .bind(&event.signal_type)
        .bind(&event.rule_id)
        .bind(&event.session_id)
        .bind(&event.tool_name)
        .bind(&event.tool_input)
        .bind(&event.override_reason)
        .bind(&event.failure_type)
        .bind(&event.evidence_url)
        .bind(&event.project_id)
        .bind(event.created_at)
        .execute(&self.pool)
        .await
        .context("inserting feedback event")?;

        Ok(())
    }

    async fn get_feedback(&self, filters: FeedbackFilters) -> Result<Vec<FeedbackEvent>> {
        let mut sql = String::from("SELECT * FROM feedback_events WHERE 1=1");
        let mut param_idx: u32 = 0;

        // Build the query string with numbered placeholders
        if filters.rule_id.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND rule_id = ${param_idx}"));
        }
        if filters.signal_type.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND signal_type = ${param_idx}"));
        }
        if filters.session_id.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND session_id = ${param_idx}"));
        }
        if filters.project_id.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND project_id = ${param_idx}"));
        }
        if filters.since.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND created_at >= ${param_idx}"));
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filters.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut query = sqlx::query_as::<_, PgFeedbackRow>(&sql);
        if let Some(ref v) = filters.rule_id {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.signal_type {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.session_id {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.project_id {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.since {
            query = query.bind(v);
        }

        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter().map(|r| r.into_event()).collect()
    }

    // -- Scoring snapshots --------------------------------------------------

    async fn record_scores(&self, scores: GovernanceScores) -> Result<()> {
        let id = scores.id.to_string();

        sqlx::query(
            "INSERT INTO scoring_snapshots (id, project_id, safety_score, friction_score, blocks_count, override_count, total_tool_calls, coverage_score, covered_categories, total_categories, converged, rule_churn_rate, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
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
        .bind(scores.created_at)
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
        let rows = if let Some(pid) = project_id {
            sqlx::query_as::<_, PgScoresRow>(
                "SELECT * FROM scoring_snapshots WHERE project_id = $1 AND created_at >= $2 ORDER BY created_at",
            )
            .bind(pid)
            .bind(since)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PgScoresRow>(
                "SELECT * FROM scoring_snapshots WHERE created_at >= $1 ORDER BY created_at",
            )
            .bind(since)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(|r| r.into_scores()).collect()
    }

    // -- Pipeline runs -------------------------------------------------------

    async fn record_pipeline_run(&self, run: PipelineRun) -> Result<()> {
        sqlx::query(
            "INSERT INTO pipeline_runs (id, prompt, branch, outcome, attempts, model, build_duration_ms, build_cost_usd, inspect_duration_ms, total_duration_ms, project_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
        .bind(run.created_at)
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
            sqlx::query_as::<_, PgPipelineRunRow>(
                "SELECT * FROM pipeline_runs WHERE project_id = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(pid)
            .bind(limit_val)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PgPipelineRunRow>(
                "SELECT * FROM pipeline_runs ORDER BY created_at DESC LIMIT $1",
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

        sqlx::query(
            "INSERT INTO probe_results (id, rule_id, strategy, probe_input, bypassed, proposed_expansion, expansion_precision_drop, expansion_approved, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&id)
        .bind(&result.rule_id)
        .bind(&result.strategy)
        .bind(&result.probe_input)
        .bind(result.bypassed)
        .bind(&result.proposed_expansion)
        .bind(result.expansion_precision_drop)
        .bind(result.expansion_approved)
        .bind(result.created_at)
        .execute(&self.pool)
        .await
        .context("inserting probe result")?;

        Ok(())
    }

    async fn get_probes(&self, rule_id: &str) -> Result<Vec<ProbeResult>> {
        let rows = sqlx::query_as::<_, PgProbeRow>(
            "SELECT * FROM probe_results WHERE rule_id = $1 ORDER BY created_at DESC",
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
            sqlx::query_as::<_, PgGovEventRow>(
                "SELECT * FROM governance_events WHERE event_type = $1 ORDER BY created_at DESC",
            )
            .bind(event_type)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PgGovEventRow>(
                "SELECT * FROM governance_events ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(|r| r.into_event()).collect()
    }

    async fn get_governance_event(&self, id: &str) -> Result<Option<GovernanceEvent>> {
        let row =
            sqlx::query_as::<_, PgGovEventRow>("SELECT * FROM governance_events WHERE id = $1")
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
        let now = Utc::now();
        let severity = event.severity.unwrap_or_else(|| "medium".to_string());
        let source = event.source.unwrap_or_else(|| "api".to_string());

        sqlx::query(
            "INSERT INTO governance_events (id, event_type, title, severity, source, metadata, resolved, created_at)
             VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, FALSE, $6)",
        )
        .bind(&id)
        .bind(&event.event_type)
        .bind(&event.title)
        .bind(&severity)
        .bind(&source)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("inserting governance event")?;

        self.get_governance_event(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("governance event not found after insert"))
    }

    async fn resolve_governance_event(&self, id: &str, notes: Option<String>) -> Result<()> {
        let now = Utc::now();

        sqlx::query(
            "UPDATE governance_events SET resolved = TRUE, resolution_notes = $1, resolved_at = $2 WHERE id = $3",
        )
        .bind(&notes)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row types — map DB rows to domain types (native Postgres types)
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct PgRuleRow {
    id: String,
    description: String,
    category_id: String,
    tools: serde_json::Value,
    condition_type: String,
    condition_value: String,
    lifecycle: String,
    alpha: i32,
    beta: i32,
    prior_alpha: i32,
    prior_beta: i32,
    enabled: bool,
    project_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    crystallized_at: Option<DateTime<Utc>>,
    cross_project_validated: bool,
}

impl PgRuleRow {
    fn into_rule(self) -> Result<Rule> {
        Ok(Rule {
            id: self.id,
            description: self.description,
            category_id: self.category_id,
            tools: serde_json::from_value(self.tools).unwrap_or_default(),
            condition_type: self.condition_type,
            condition_value: self.condition_value,
            lifecycle: self.lifecycle.parse()?,
            alpha: self.alpha,
            beta: self.beta,
            prior_alpha: self.prior_alpha,
            prior_beta: self.prior_beta,
            enabled: self.enabled,
            project_id: self.project_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            crystallized_at: self.crystallized_at,
            cross_project_validated: self.cross_project_validated,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgCategoryRow {
    id: String,
    name: String,
    description: String,
    severity: String,
    severity_weight: f64,
    examples: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl PgCategoryRow {
    fn into_category(self) -> Result<ThreatCategory> {
        Ok(ThreatCategory {
            id: self.id,
            name: self.name,
            description: self.description,
            severity: self.severity,
            severity_weight: self.severity_weight,
            examples: serde_json::from_value(self.examples).unwrap_or_default(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgFeedbackRow {
    id: String,
    signal_type: String,
    rule_id: String,
    session_id: Option<String>,
    tool_name: String,
    tool_input: serde_json::Value,
    override_reason: Option<String>,
    failure_type: Option<String>,
    evidence_url: Option<String>,
    project_id: Option<String>,
    created_at: DateTime<Utc>,
}

impl PgFeedbackRow {
    fn into_event(self) -> Result<FeedbackEvent> {
        Ok(FeedbackEvent {
            id: Uuid::parse_str(&self.id)?,
            signal_type: self.signal_type,
            rule_id: self.rule_id,
            session_id: self.session_id,
            tool_name: self.tool_name,
            tool_input: self.tool_input,
            override_reason: self.override_reason,
            failure_type: self.failure_type,
            evidence_url: self.evidence_url,
            project_id: self.project_id,
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgScoresRow {
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
    created_at: DateTime<Utc>,
}

impl PgScoresRow {
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
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgProbeRow {
    id: String,
    rule_id: String,
    strategy: String,
    probe_input: serde_json::Value,
    bypassed: bool,
    proposed_expansion: Option<String>,
    expansion_precision_drop: Option<f64>,
    expansion_approved: Option<bool>,
    created_at: DateTime<Utc>,
}

impl PgProbeRow {
    fn into_probe(self) -> Result<ProbeResult> {
        Ok(ProbeResult {
            id: Uuid::parse_str(&self.id)?,
            rule_id: self.rule_id,
            strategy: self.strategy,
            probe_input: self.probe_input,
            bypassed: self.bypassed,
            proposed_expansion: self.proposed_expansion,
            expansion_precision_drop: self.expansion_precision_drop,
            expansion_approved: self.expansion_approved,
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgPipelineRunRow {
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
    created_at: DateTime<Utc>,
}

impl PgPipelineRunRow {
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
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PgGovEventRow {
    id: String,
    event_type: String,
    title: String,
    severity: String,
    source: String,
    metadata: serde_json::Value,
    resolved: bool,
    resolution_notes: Option<String>,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
}

impl PgGovEventRow {
    fn into_event(self) -> Result<GovernanceEvent> {
        Ok(GovernanceEvent {
            id: self.id,
            event_type: self.event_type,
            title: self.title,
            severity: self.severity,
            source: self.source,
            metadata: self.metadata,
            resolved: self.resolved,
            resolution_notes: self.resolution_notes,
            created_at: self.created_at,
            resolved_at: self.resolved_at,
        })
    }
}
