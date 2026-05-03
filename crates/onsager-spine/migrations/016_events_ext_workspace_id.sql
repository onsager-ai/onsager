-- Onsager #183 — events_ext.workspace_id as a real column.
--
-- Until now extension events stamped workspace scope into the JSONB
-- payload (data->>'workspace_id'). PR #232 made every forge stage
-- event carry that field; the dashboard's `/api/spine/events` filter
-- has been a JSONB predicate. JSONB lookup is unindexed at our row
-- counts and silently drops events whose payload is missing the field.
--
-- Promote workspace_id to a first-class column on `events_ext`, with
-- a 3-stage backfill so the column can carry NOT NULL by the end of
-- the migration:
--
--   1. ADD COLUMN nullable.
--   2. UPDATE rows in place — prefer the existing JSONB hint when set,
--      else 'default' (a few legacy rows from before #230 carry no
--      tenant; system events never will).
--   3. ALTER COLUMN ... SET NOT NULL DEFAULT 'default'.
--
-- After the migration every events_ext row carries workspace_id; the
-- dashboard read at `crates/stiglab/src/server/routes/spine.rs` swaps
-- its `data->>'workspace_id' = $1` predicate for `workspace_id = $1`
-- and the composite index below covers the read pattern.
--
-- Out of scope:
--   - `events` (factory event) table — this spec is events_ext only.
--   - FK to `workspaces(id)` — that's a separate spec when the
--     `workspaces` table lives definitively in spine vs. portal.

-- Stage 1: add the column nullable so the backfill UPDATE can run.
ALTER TABLE events_ext
    ADD COLUMN IF NOT EXISTS workspace_id TEXT;

-- Stage 2: backfill. Prefer the JSONB-stamped value (PR #230 / #232),
-- fall back to 'default' for legacy or system rows that never had a
-- tenant scope.
UPDATE events_ext
SET workspace_id = COALESCE(NULLIF(data->>'workspace_id', ''), 'default')
WHERE workspace_id IS NULL;

-- Stage 3: lock the contract — every row has a tenant, default 'default'.
ALTER TABLE events_ext
    ALTER COLUMN workspace_id SET NOT NULL,
    ALTER COLUMN workspace_id SET DEFAULT 'default';

-- Composite index supports the dashboard's per-workspace stream read
-- (`WHERE workspace_id = $1 ORDER BY id DESC`); leading on workspace_id
-- prunes to a tenant first, then `id DESC` falls out as a covered scan.
CREATE INDEX IF NOT EXISTS idx_events_ext_workspace_id
    ON events_ext (workspace_id, id DESC);
