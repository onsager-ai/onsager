-- Pipeline telemetry — track pipeline run outcomes and durations (PostgreSQL)
-- Spec 078: Wire synodic run events to governance storage layer

CREATE TABLE IF NOT EXISTS pipeline_runs (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    branch TEXT,
    outcome TEXT NOT NULL CHECK (outcome IN ('passed', 'failed', 'error')),
    attempts INTEGER NOT NULL,
    model TEXT,
    build_duration_ms BIGINT,
    build_cost_usd DOUBLE PRECISION,
    inspect_duration_ms BIGINT,
    total_duration_ms BIGINT NOT NULL,
    project_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_created ON pipeline_runs(created_at);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_outcome ON pipeline_runs(outcome);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_project_created ON pipeline_runs(project_id, created_at);
