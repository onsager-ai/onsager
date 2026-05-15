-- Onsager 0.2 substrate — issue #348 (SUB-01).
--
-- ADR 0010 makes provenance a first-class field on every artifact:
-- the substrate needs to answer, given an artifact, whether a
-- downstream consumer may trust it as deterministic. Without this
-- column, the kernel invariants formalized in ADR 0018 (especially
-- invariants 1 and 2) cannot be statically validated.
--
-- This migration:
--   1. Adds `provenance` (jsonb) carrying the kernel-recognized
--      `Provenance` enum's wire form (`{"kind": "...", "source": "..."}`).
--      NOT NULL with a default — existing rows backfill to
--      `Deterministic { source: External }` because they were
--      produced before workflow runtime existed, so their authority
--      is the external system-of-record (GitHub, the dashboard, an
--      ingestion job), not an agent.
--   2. Adds `produced_by_node` (uuid, nullable) — populated by the
--      workflow runtime at dispatch time. NULL for legacy /
--      externally-ingested rows. References the node module that
--      lands in SUB-02 (#349); no FK yet — the `nodes` table itself
--      is introduced there.
--
-- Pre-launch posture (CLAUDE.md § "Operating posture: pre-launch")
-- skips deprecation choreography: the column is NOT NULL from day one
-- with a default + inline backfill in a single transaction.

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS provenance JSONB NOT NULL
        DEFAULT '{"kind": "deterministic", "source": "external"}'::jsonb;

ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS produced_by_node UUID;

-- Re-state the backfill explicitly so re-runs against a partially-
-- migrated database converge — `ADD COLUMN ... DEFAULT` only fires
-- on the first add, but an `IF NOT EXISTS` no-op leaves nothing for
-- the default to do. The `WHERE` clause makes this a no-op on a
-- freshly-populated default.
UPDATE artifacts
   SET provenance = '{"kind": "deterministic", "source": "external"}'::jsonb
 WHERE provenance IS NULL
    OR provenance = '{}'::jsonb;

-- Filtering by provenance kind ("show me uncertain artifacts") is a
-- recurring query for the dashboard and the static validators. A
-- jsonb_path_ops GIN index covers the equality lookups on `kind` /
-- `source` without bloating like a btree on the whole jsonb would.
CREATE INDEX IF NOT EXISTS idx_artifacts_provenance
    ON artifacts USING gin (provenance jsonb_path_ops);

CREATE INDEX IF NOT EXISTS idx_artifacts_produced_by_node
    ON artifacts (produced_by_node)
    WHERE produced_by_node IS NOT NULL;
