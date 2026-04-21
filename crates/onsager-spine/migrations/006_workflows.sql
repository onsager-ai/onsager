-- Onsager Level 5: Workflow runtime (issue #80)
-- See issue #80 and parent #79 for the data model specification.
--
-- Adds two concepts:
--   1. workflows        — declarative production-line blueprints (trigger + stage chain).
--   2. workflow_stages  — ordered stages per workflow, each with its own gate set.
--
-- Also augments artifacts with workflow_id / current_stage_index / workflow_parked_reason
-- so the stage runner can resume progress across restarts.

-- -- Workflows -----------------------------------------------------------------
CREATE TABLE IF NOT EXISTS workflows (
    workflow_id             TEXT PRIMARY KEY,
    name                    TEXT NOT NULL,
    trigger_kind            TEXT NOT NULL
        CHECK (trigger_kind IN ('github_issue_webhook')),
    trigger_config          JSONB NOT NULL DEFAULT '{}'::jsonb,
    active                  BOOLEAN NOT NULL DEFAULT FALSE,
    preset_id               TEXT,
    workspace_install_ref   TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflows_active ON workflows (active);
CREATE INDEX IF NOT EXISTS idx_workflows_trigger_kind ON workflows (trigger_kind);

-- -- Workflow stages -----------------------------------------------------------
-- Each stage is one step in the workflow chain. stage_order is 0-based; a
-- stage's gates must all resolve before the runner advances past it.
-- target_state maps to ArtifactState and is optional — a stage can share the
-- previous stage's state (e.g. multiple gates on the same UnderReview state).
CREATE TABLE IF NOT EXISTS workflow_stages (
    workflow_id   TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    stage_order   INTEGER NOT NULL,
    name          TEXT NOT NULL,
    target_state  TEXT
        CHECK (target_state IS NULL OR target_state IN
               ('draft', 'in_progress', 'under_review', 'released', 'archived')),
    gates         JSONB NOT NULL DEFAULT '[]'::jsonb,
    params        JSONB NOT NULL DEFAULT '{}'::jsonb,

    PRIMARY KEY (workflow_id, stage_order)
);

CREATE INDEX IF NOT EXISTS idx_workflow_stages_workflow
    ON workflow_stages (workflow_id, stage_order);

-- -- Artifact augmentation -----------------------------------------------------
-- workflow_id tags an artifact with the workflow driving it.
-- current_stage_index is the 0-based index of the stage the artifact is in.
-- workflow_parked_reason explains why advancement is blocked (e.g. CI failure).
ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS workflow_id             TEXT REFERENCES workflows(workflow_id),
    ADD COLUMN IF NOT EXISTS current_stage_index     INTEGER,
    ADD COLUMN IF NOT EXISTS workflow_parked_reason  TEXT;

CREATE INDEX IF NOT EXISTS idx_artifacts_workflow_id ON artifacts (workflow_id);

-- -- Refresh workflows.updated_at on UPDATE
CREATE OR REPLACE FUNCTION update_workflow_timestamp() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS workflow_updated ON workflows;
CREATE TRIGGER workflow_updated BEFORE UPDATE ON workflows
    FOR EACH ROW EXECUTE FUNCTION update_workflow_timestamp();
