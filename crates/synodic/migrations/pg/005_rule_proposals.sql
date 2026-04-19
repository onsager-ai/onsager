-- Rule-proposal queue fed by Ising (issue #36 Step 2, PostgreSQL).
--
-- See the SQLite sibling migration (`005_rule_proposals.sql`) for design
-- notes — this file is the production schema.

CREATE TABLE IF NOT EXISTS rule_proposals (
    id TEXT PRIMARY KEY,
    insight_id TEXT NOT NULL UNIQUE,
    signal_kind TEXT NOT NULL,
    subject_ref TEXT NOT NULL,
    proposed_action JSONB NOT NULL,
    class TEXT NOT NULL CHECK (class IN ('safe_auto', 'review_required')),
    rationale TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    resolution_notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_proposals_status ON rule_proposals(status);
CREATE INDEX IF NOT EXISTS idx_proposals_subject ON rule_proposals(subject_ref);
CREATE INDEX IF NOT EXISTS idx_proposals_created ON rule_proposals(created_at);
