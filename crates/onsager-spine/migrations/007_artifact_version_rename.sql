-- Issue #100 / #101: rename the artifact-level warehouse pointer fields
-- from `bundle`-prefixed names to `version`-prefixed names. The name
-- `BundleId` implied "workflow output container" but the column actually
-- stores a per-artifact version snapshot id; the ADR 0003 / #100 redesign
-- introduces a separate `Deliverable` for the aggregate concept.
--
-- The `bundles` / `deliveries` tables in migration 005 stay under their
-- current names — they're the warehouse's storage model and rename is out
-- of scope for this migration.

-- Rename the two columns on `artifacts` added in migration 005.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'artifacts' AND column_name = 'current_bundle_id'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'artifacts' AND column_name = 'current_version_id'
    ) THEN
        ALTER TABLE artifacts RENAME COLUMN current_bundle_id TO current_version_id;
    END IF;
END;
$$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'artifacts' AND column_name = 'bundle_history'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'artifacts' AND column_name = 'version_history'
    ) THEN
        ALTER TABLE artifacts RENAME COLUMN bundle_history TO version_history;
    END IF;
END;
$$;

-- The FK constraint on `current_bundle_id` (added in 005) keeps its
-- original name via PostgreSQL's column-rename semantics — the constraint
-- references the column by attnum, not by textual name. Rename it to match
-- the new column so `pg_constraint` is legible.
ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_current_bundle_fkey;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'artifacts_current_version_fkey'
    ) THEN
        ALTER TABLE artifacts
            ADD CONSTRAINT artifacts_current_version_fkey
            FOREIGN KEY (current_version_id) REFERENCES bundles(bundle_id);
    END IF;
END;
$$;
