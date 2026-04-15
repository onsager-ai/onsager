-- Governance data model — initial schema (PostgreSQL)

CREATE TABLE IF NOT EXISTS threat_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    severity_weight DOUBLE PRECISION NOT NULL,
    examples JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rules (
    id TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    category_id TEXT NOT NULL REFERENCES threat_categories(id),
    tools JSONB NOT NULL DEFAULT '[]',
    condition_type TEXT NOT NULL CHECK (condition_type IN ('pattern', 'path', 'command')),
    condition_value TEXT NOT NULL,
    lifecycle TEXT NOT NULL DEFAULT 'candidate' CHECK (lifecycle IN ('candidate', 'active', 'tuned', 'crystallized', 'deprecated')),

    -- Bayesian confidence tracking
    alpha INTEGER NOT NULL DEFAULT 1,
    beta INTEGER NOT NULL DEFAULT 1,
    prior_alpha INTEGER NOT NULL DEFAULT 1,
    prior_beta INTEGER NOT NULL DEFAULT 1,

    -- Metadata
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    project_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Crystallization metadata
    crystallized_at TIMESTAMPTZ,
    cross_project_validated BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_rules_category ON rules(category_id);
CREATE INDEX IF NOT EXISTS idx_rules_lifecycle ON rules(lifecycle);
CREATE INDEX IF NOT EXISTS idx_rules_enabled ON rules(enabled);

CREATE TABLE IF NOT EXISTS feedback_events (
    id TEXT PRIMARY KEY,
    signal_type TEXT NOT NULL CHECK (signal_type IN ('override', 'confirmed', 'ci_pass', 'ci_failure', 'incident')),
    rule_id TEXT NOT NULL REFERENCES rules(id),
    session_id TEXT,
    tool_name TEXT NOT NULL,
    tool_input JSONB NOT NULL DEFAULT '{}',
    override_reason TEXT,
    failure_type TEXT,
    evidence_url TEXT,
    project_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_feedback_rule ON feedback_events(rule_id);
CREATE INDEX IF NOT EXISTS idx_feedback_signal ON feedback_events(signal_type);
CREATE INDEX IF NOT EXISTS idx_feedback_session ON feedback_events(session_id);
CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback_events(created_at);

CREATE TABLE IF NOT EXISTS scoring_snapshots (
    id TEXT PRIMARY KEY,
    project_id TEXT,
    safety_score DOUBLE PRECISION NOT NULL,
    friction_score DOUBLE PRECISION NOT NULL,
    blocks_count INTEGER NOT NULL,
    override_count INTEGER NOT NULL,
    total_tool_calls INTEGER NOT NULL,
    coverage_score DOUBLE PRECISION NOT NULL,
    covered_categories INTEGER NOT NULL,
    total_categories INTEGER NOT NULL,
    converged BOOLEAN NOT NULL DEFAULT FALSE,
    rule_churn_rate DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scores_project ON scoring_snapshots(project_id);
CREATE INDEX IF NOT EXISTS idx_scores_created ON scoring_snapshots(created_at);

CREATE TABLE IF NOT EXISTS probe_results (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES rules(id),
    strategy TEXT NOT NULL,
    probe_input JSONB NOT NULL DEFAULT '{}',
    bypassed BOOLEAN NOT NULL DEFAULT FALSE,
    proposed_expansion TEXT,
    expansion_precision_drop DOUBLE PRECISION,
    expansion_approved BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_probe_rule ON probe_results(rule_id);
CREATE INDEX IF NOT EXISTS idx_probe_bypassed ON probe_results(bypassed);
