-- Onsager #183 — events_ext.workspace_id as a real column.
--
-- Until now extension events stamped workspace scope into the JSONB
-- payload (data->>'workspace_id'). PR #232 made every forge stage
-- event carry that field; the dashboard's `/api/spine/events` filter
-- has been a JSONB predicate. JSONB lookup is unindexed at our row
-- counts and silently drops events whose payload is missing the field.
--
-- Promote workspace_id to a first-class column on `events_ext`. The
-- migration is rolling-deploy safe: the column lands NOT NULL DEFAULT
-- 'default' on stage 1, so older writers that don't yet bind the
-- column can still INSERT — they pick up the default. The backfill in
-- stage 2 then rewrites pre-existing rows from the JSONB hint when
-- present.
--
-- (An earlier draft staged ADD nullable → UPDATE → SET NOT NULL.
-- Copilot review on PR #235 flagged the race: a concurrent INSERT
-- between the UPDATE and the SET NOT NULL would land workspace_id =
-- NULL and break the final ALTER. Setting the default at stage 1
-- closes that window.)
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

-- Stage 1: add the column NOT NULL with a default so concurrent
-- writers can't land NULL during the rest of this migration.
ALTER TABLE events_ext
    ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default';

-- Stage 2: backfill pre-existing rows that carried the JSONB hint
-- (PR #230 / #232) but were stamped 'default' by stage 1. Rows that
-- never had a tenant — legacy / system events — keep 'default'.
UPDATE events_ext
SET workspace_id = data->>'workspace_id'
WHERE workspace_id = 'default'
  AND data ? 'workspace_id'
  AND data->>'workspace_id' <> ''
  AND data->>'workspace_id' <> 'default';

-- Composite index supports the dashboard's per-workspace stream read
-- (`WHERE workspace_id = $1 ORDER BY id DESC`); leading on workspace_id
-- prunes to a tenant first, then `id DESC` falls out as a covered scan.
CREATE INDEX IF NOT EXISTS idx_events_ext_workspace_id
    ON events_ext (workspace_id, id DESC);
