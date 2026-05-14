-- Workflow versioning substrate — spec #337.
--
-- Workflow content (trigger + stage chain + future composition fields)
-- moves from spread-across `workflows`/`workflow_stages` to a per-version
-- JSONB snapshot. The `workflows` row keeps identity, lifecycle hooks
-- (active flag, install/workspace scope), and a pointer to the active
-- version. Stage rows continue to drive forge's stage runner; the JSONB
-- snapshot is the source of truth for export/import and audit diffs.
--
-- v1 published versions are backfilled by migration 025 from the
-- existing `workflows` + `workflow_stages` content so no behavior
-- changes at the read path.

CREATE TABLE IF NOT EXISTS workflow_versions (
    version_id          TEXT PRIMARY KEY,
    workflow_id         TEXT NOT NULL REFERENCES workflows(workflow_id) ON DELETE CASCADE,
    version_label       TEXT NOT NULL,
    content             JSONB NOT NULL,
    parent_version_id   TEXT REFERENCES workflow_versions(version_id),
    state               TEXT NOT NULL
        CHECK (state IN ('draft', 'published', 'retired')),
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at        TIMESTAMPTZ,
    UNIQUE (workflow_id, version_label)
);

CREATE INDEX IF NOT EXISTS idx_workflow_versions_workflow
    ON workflow_versions (workflow_id, state);
