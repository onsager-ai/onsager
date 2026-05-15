-- Onsager 0.2 substrate — issue #351 (SUB-04).
--
-- ADR 0016 makes the Workflow Library the flat catalog that maps a
-- spec kind to its reusable `Workflow` template. The substrate kernel
-- (issue #349) defined the `Workflow` value object; this migration
-- adds the persistence layer the Plan Compiler (SUB-05, #352) will
-- read from when it instantiates a Spec Plan into an Execution Plan.
--
-- Per the issue's storage spec: one row per (spec_kind, version).
-- Versions are monotonic per kind; the latest version wins (`latest()`
-- uses `MAX(version)`). The unique constraint on (spec_kind, version)
-- is what surfaces the substrate's `DuplicateKind` error — two
-- workflows at the same (kind, version) are rejected at the database.
--
-- Pre-launch posture (CLAUDE.md § "Operating posture: pre-launch") —
-- this table is brand-new and has no backfill obligations.

CREATE TABLE IF NOT EXISTS workflow_library (
    id              TEXT PRIMARY KEY,
    spec_kind       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    workflow_json   JSONB NOT NULL,
    registered_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (spec_kind, version)
);

-- `latest()` picks `MAX(version)` per kind; an index ordered by
-- `(spec_kind, version DESC)` lets the planner answer that with a
-- straight index scan instead of a sort.
CREATE INDEX IF NOT EXISTS idx_workflow_library_kind_version
    ON workflow_library (spec_kind, version DESC);
