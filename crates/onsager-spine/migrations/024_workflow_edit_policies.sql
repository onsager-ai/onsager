-- Per-workspace workflow edit policy — spec #337.
--
-- Two modes:
--   * `direct`               — chat edits write and publish in one step;
--                              suitable for personal / dev workspaces.
--   * `draft_then_publish`   — chat edits create a draft version; an
--                              explicit publish promotes the draft to
--                              active. Default for new workspaces.
--
-- A workspace's policy may not be weakened while any of its workflows
-- have in-flight runs (enforced at the application layer).

CREATE TABLE IF NOT EXISTS workflow_edit_policies (
    workspace_id        TEXT PRIMARY KEY,
    edit_mode           TEXT NOT NULL DEFAULT 'draft_then_publish'
        CHECK (edit_mode IN ('direct', 'draft_then_publish')),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
