-- Synodic governance schema (PostgreSQL).
--
-- threat_categories — severity taxonomy for classifying rule violations.
-- rules             — safety rules with Bayesian confidence tracking.
-- feedback_events   — signal data that drives rule confidence updates.
-- scoring_snapshots — periodic telemetry snapshots (safety/friction scores).
-- probe_results     — rule expansion probe outcomes.
-- pipeline_runs     — pipeline telemetry (outcome, duration, cost).
-- governance_events — dashboard event model (alerts, incidents, resolutions).
-- rule_proposals    — Ising-generated rule proposals pending human review.

CREATE TABLE IF NOT EXISTS threat_categories (
    id               TEXT             NOT NULL PRIMARY KEY,
    name             TEXT             NOT NULL,
    description      TEXT             NOT NULL,
    severity         TEXT             NOT NULL
        CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    severity_weight  DOUBLE PRECISION NOT NULL,
    examples         JSONB            NOT NULL DEFAULT '[]',
    created_at       TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rules (
    id                       TEXT             NOT NULL PRIMARY KEY,
    description              TEXT             NOT NULL,
    category_id              TEXT             NOT NULL REFERENCES threat_categories(id),
    tools                    JSONB            NOT NULL DEFAULT '[]',
    condition_type           TEXT             NOT NULL
        CHECK (condition_type IN ('pattern', 'path', 'command')),
    condition_value          TEXT             NOT NULL,
    lifecycle                TEXT             NOT NULL DEFAULT 'candidate'
        CHECK (lifecycle IN ('candidate', 'active', 'tuned', 'crystallized', 'deprecated')),
    -- Bayesian confidence tracking (alpha/beta are posterior parameters).
    alpha                    INTEGER          NOT NULL DEFAULT 1,
    beta                     INTEGER          NOT NULL DEFAULT 1,
    prior_alpha              INTEGER          NOT NULL DEFAULT 1,
    prior_beta               INTEGER          NOT NULL DEFAULT 1,
    enabled                  BOOLEAN          NOT NULL DEFAULT TRUE,
    project_id               TEXT,
    created_at               TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    crystallized_at          TIMESTAMPTZ,
    cross_project_validated  BOOLEAN          NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_rules_category  ON rules (category_id);
CREATE INDEX IF NOT EXISTS idx_rules_lifecycle ON rules (lifecycle);
CREATE INDEX IF NOT EXISTS idx_rules_enabled   ON rules (enabled);

CREATE TABLE IF NOT EXISTS feedback_events (
    id              TEXT        NOT NULL PRIMARY KEY,
    signal_type     TEXT        NOT NULL
        CHECK (signal_type IN ('override', 'confirmed', 'ci_pass', 'ci_failure', 'incident')),
    rule_id         TEXT        NOT NULL REFERENCES rules(id),
    session_id      TEXT,
    tool_name       TEXT        NOT NULL,
    tool_input      JSONB       NOT NULL DEFAULT '{}',
    override_reason TEXT,
    failure_type    TEXT,
    evidence_url    TEXT,
    project_id      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_feedback_rule    ON feedback_events (rule_id);
CREATE INDEX IF NOT EXISTS idx_feedback_signal  ON feedback_events (signal_type);
CREATE INDEX IF NOT EXISTS idx_feedback_session ON feedback_events (session_id);
CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback_events (created_at);

CREATE TABLE IF NOT EXISTS scoring_snapshots (
    id               TEXT             NOT NULL PRIMARY KEY,
    project_id       TEXT,
    safety_score     DOUBLE PRECISION NOT NULL,
    friction_score   DOUBLE PRECISION NOT NULL,
    blocks_count     INTEGER          NOT NULL,
    override_count   INTEGER          NOT NULL,
    total_tool_calls INTEGER          NOT NULL,
    coverage_score   DOUBLE PRECISION NOT NULL,
    covered_categories INTEGER        NOT NULL,
    total_categories INTEGER          NOT NULL,
    converged        BOOLEAN          NOT NULL DEFAULT FALSE,
    rule_churn_rate  DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at       TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scores_project ON scoring_snapshots (project_id);
CREATE INDEX IF NOT EXISTS idx_scores_created ON scoring_snapshots (created_at);

CREATE TABLE IF NOT EXISTS probe_results (
    id                       TEXT             NOT NULL PRIMARY KEY,
    rule_id                  TEXT             NOT NULL REFERENCES rules(id),
    strategy                 TEXT             NOT NULL,
    probe_input              JSONB            NOT NULL DEFAULT '{}',
    bypassed                 BOOLEAN          NOT NULL DEFAULT FALSE,
    proposed_expansion       TEXT,
    expansion_precision_drop DOUBLE PRECISION,
    expansion_approved       BOOLEAN,
    created_at               TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_probe_rule     ON probe_results (rule_id);
CREATE INDEX IF NOT EXISTS idx_probe_bypassed ON probe_results (bypassed);

CREATE TABLE IF NOT EXISTS pipeline_runs (
    id                  TEXT             NOT NULL PRIMARY KEY,
    prompt              TEXT             NOT NULL,
    branch              TEXT,
    outcome             TEXT             NOT NULL
        CHECK (outcome IN ('passed', 'failed', 'error')),
    attempts            INTEGER          NOT NULL,
    model               TEXT,
    build_duration_ms   BIGINT,
    build_cost_usd      DOUBLE PRECISION,
    inspect_duration_ms BIGINT,
    total_duration_ms   BIGINT           NOT NULL,
    project_id          TEXT,
    created_at          TIMESTAMPTZ      NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_created         ON pipeline_runs (created_at);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_outcome         ON pipeline_runs (outcome);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_project_created ON pipeline_runs (project_id, created_at);

CREATE TABLE IF NOT EXISTS governance_events (
    id               TEXT        NOT NULL PRIMARY KEY,
    event_type       TEXT        NOT NULL,
    title            TEXT        NOT NULL,
    severity         TEXT        NOT NULL DEFAULT 'medium'
        CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    source           TEXT        NOT NULL DEFAULT 'system',
    metadata         JSONB       NOT NULL DEFAULT '{}',
    resolved         BOOLEAN     NOT NULL DEFAULT FALSE,
    resolution_notes TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_govevents_type     ON governance_events (event_type);
CREATE INDEX IF NOT EXISTS idx_govevents_severity ON governance_events (severity);
CREATE INDEX IF NOT EXISTS idx_govevents_resolved ON governance_events (resolved);
CREATE INDEX IF NOT EXISTS idx_govevents_created  ON governance_events (created_at);

-- Rule-proposal queue fed by Ising (spec #36 Step 2).
CREATE TABLE IF NOT EXISTS rule_proposals (
    id               TEXT             NOT NULL PRIMARY KEY,
    insight_id       TEXT             NOT NULL UNIQUE,
    signal_kind      TEXT             NOT NULL,
    subject_ref      TEXT             NOT NULL,
    proposed_action  JSONB            NOT NULL,
    class            TEXT             NOT NULL
        CHECK (class IN ('safe_auto', 'review_required')),
    rationale        TEXT             NOT NULL,
    confidence       DOUBLE PRECISION NOT NULL,
    status           TEXT             NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected')),
    resolution_notes TEXT,
    created_at       TIMESTAMPTZ      NOT NULL DEFAULT NOW(),
    resolved_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_proposals_status  ON rule_proposals (status);
CREATE INDEX IF NOT EXISTS idx_proposals_subject ON rule_proposals (subject_ref);
CREATE INDEX IF NOT EXISTS idx_proposals_created ON rule_proposals (created_at);
