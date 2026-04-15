-- Governance events table — dashboard event model
-- Used by the web dashboard to display governance events

CREATE TABLE IF NOT EXISTS governance_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    title TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'medium' CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    source TEXT NOT NULL DEFAULT 'system',
    metadata TEXT NOT NULL DEFAULT '{}',     -- JSON object
    resolved INTEGER NOT NULL DEFAULT 0,
    resolution_notes TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_govevents_type ON governance_events(event_type);
CREATE INDEX IF NOT EXISTS idx_govevents_severity ON governance_events(severity);
CREATE INDEX IF NOT EXISTS idx_govevents_resolved ON governance_events(resolved);
CREATE INDEX IF NOT EXISTS idx_govevents_created ON governance_events(created_at);
