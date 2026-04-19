-- Rule-proposal queue fed by Ising (issue #36 Step 2).
--
-- Each row is one ising rule-proposed event ingested by Synodic. Rows of
-- class review_required stay at status pending until a human or agent
-- resolves them. Rows of class safe_auto auto-transition to approved and
-- apply the rule change inline.
--
-- insight_id is UNIQUE so redelivery (listener restart, catch-up) is
-- idempotent: the listener INSERTs with ON CONFLICT DO NOTHING and the
-- earliest handler wins.

CREATE TABLE IF NOT EXISTS rule_proposals (
    id TEXT PRIMARY KEY,                          -- internal UUID
    insight_id TEXT NOT NULL UNIQUE,              -- dedup key from ising
    signal_kind TEXT NOT NULL,                    -- e.g. "repeated_gate_override"
    subject_ref TEXT NOT NULL,                    -- artifact kind / rule id / ...
    proposed_action TEXT NOT NULL,                -- JSON-encoded RuleProposalAction
    class TEXT NOT NULL CHECK (class IN ('safe_auto', 'review_required')),
    rationale TEXT NOT NULL,
    confidence REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    resolution_notes TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_proposals_status ON rule_proposals(status);
CREATE INDEX IF NOT EXISTS idx_proposals_subject ON rule_proposals(subject_ref);
CREATE INDEX IF NOT EXISTS idx_proposals_created ON rule_proposals(created_at);
