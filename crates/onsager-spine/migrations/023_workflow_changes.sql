-- Workflow audit trail — spec #337.
--
-- Every state-changing operation on a workflow writes one row here:
-- draft creation, draft edit, publish, retire, revert. `before_content`
-- and `after_content` snapshots make diff rendering server-side trivial
-- without re-joining version history.
--
-- The same table is the natural input for an Ising consumer that wants
-- to surface workflow churn ("edited 40 times this week, signal: churn")
-- without code changes — the audit row is the durable record.

CREATE TABLE IF NOT EXISTS workflow_changes (
    change_id           BIGSERIAL PRIMARY KEY,
    workflow_id         TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    version_id          TEXT REFERENCES workflow_versions(version_id) ON DELETE SET NULL,
    actor               TEXT NOT NULL,
    action              TEXT NOT NULL
        CHECK (action IN ('create_draft', 'edit_draft', 'publish', 'retire', 'revert')),
    before_content      JSONB,
    after_content       JSONB,
    reason              TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_changes_workflow
    ON workflow_changes (workflow_id, created_at DESC);
