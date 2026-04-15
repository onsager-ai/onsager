-- Governance events table — dashboard event model (PostgreSQL)

CREATE TABLE IF NOT EXISTS governance_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    title TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'medium' CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    source TEXT NOT NULL DEFAULT 'system',
    metadata JSONB NOT NULL DEFAULT '{}',
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolution_notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_govevents_type ON governance_events(event_type);
CREATE INDEX IF NOT EXISTS idx_govevents_severity ON governance_events(severity);
CREATE INDEX IF NOT EXISTS idx_govevents_resolved ON governance_events(resolved);
CREATE INDEX IF NOT EXISTS idx_govevents_created ON governance_events(created_at);
