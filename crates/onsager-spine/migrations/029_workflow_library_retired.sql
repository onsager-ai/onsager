-- Onsager #395 — 0.2 substrate authoring tools via MCP.
--
-- Adds a `retired_at` timestamp to `workflow_library` so the MCP
-- `retire_workflow` tool can mark an active workflow inactive without
-- registering a tombstone row.
--
-- ADR 0016 fixes "one active workflow per kind: the latest version
-- wins". A retire operation marks the current latest row inactive,
-- meaning a fresh `submit_workflow` is needed to re-establish an
-- active workflow for that kind. The substrate compile path (and the
-- portal-side `latest_active` query) read `WHERE retired_at IS NULL`,
-- so a retired row stops being considered for compilation while the
-- audit history of every previously-registered version stays intact.
--
-- Pre-launch posture (CLAUDE.md) — no backfill, no double-write
-- choreography, no compatibility shim. Existing rows get NULL and
-- are treated as active.

ALTER TABLE workflow_library
    ADD COLUMN IF NOT EXISTS retired_at TIMESTAMPTZ NULL;

-- Partial index for the hot path: "the latest active version for a
-- kind". Same backward-index trick as the existing unique constraint,
-- but filtered to non-retired rows so retired tail rows don't bloat
-- the lookup.
CREATE INDEX IF NOT EXISTS idx_workflow_library_active_kind
    ON workflow_library (spec_kind, version DESC)
    WHERE retired_at IS NULL;
