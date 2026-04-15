-- Pipeline telemetry — track pipeline run outcomes and durations
-- Spec 078: Wire synodic run events to governance storage layer

CREATE TABLE IF NOT EXISTS pipeline_runs (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    branch TEXT,
    outcome TEXT NOT NULL CHECK (outcome IN ('passed', 'failed', 'error')),
    attempts INTEGER NOT NULL,
    model TEXT,
    build_duration_ms INTEGER,
    build_cost_usd REAL,
    inspect_duration_ms INTEGER,
    total_duration_ms INTEGER NOT NULL,
    project_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_created ON pipeline_runs(created_at);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_outcome ON pipeline_runs(outcome);
