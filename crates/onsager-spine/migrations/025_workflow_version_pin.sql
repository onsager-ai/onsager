-- Workflow version pinning — spec #337.
--
-- Pre-launch posture (CLAUDE.md § "Operating posture: pre-launch")
-- skips deprecation choreography: backfill v1 for every existing
-- workflow inline, point `workflows.active_version_id` at it, and add
-- `artifacts.workflow_version_id` so the in-flight pinning required
-- by the spec's cybernetic invariant ("no in-flight scaffold mutation")
-- is structural.
--
-- The spec calls the column `workflow_runs.workflow_version_id`, but
-- "runs" in this monorepo are not a separate table — each artifact
-- flowing through a workflow is one run, projected from `artifacts`
-- (see `crates/onsager-portal/src/handlers/workflows.rs::list_workflow_runs`).
-- The natural mapping is therefore a column on `artifacts`.
--
-- The version-pin column starts nullable so the backfill can run in
-- a single transaction without an interim NOT NULL violation; the
-- application layer (portal's stage runner / activation path) will
-- populate it at dispatch time for new runs, and a follow-up spec can
-- tighten the constraint once every active artifact carries a value.

-- 1. Workflow pointer to active version. Nullable transitionally — the
--    backfill below points every existing row at its v1 version, but
--    new rows go through the application layer which sets the pointer
--    explicitly.
ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS active_version_id TEXT REFERENCES workflow_versions(version_id);

-- 2. Per-artifact pinned version. Nullable so existing rows survive the
--    migration; new artifact registrations through the workflow
--    activation path will populate it.
--
--    `ON DELETE SET NULL` mirrors the same posture migration 015 set on
--    `artifacts.workflow_id`. Without it, deleting a workflow would
--    cascade-delete its `workflow_versions` rows (FK from migration 022)
--    which the artifacts FK would then refuse — recreating the FK-delete
--    deadlock #233 fixed for the workflow-id column.
ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS workflow_version_id TEXT
        REFERENCES workflow_versions(version_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_artifacts_workflow_version
    ON artifacts (workflow_version_id);

-- 3. Backfill: for every workflow without an active_version_id, build a
--    canonical v1 content snapshot from its current trigger + stages,
--    insert a published `workflow_versions` row, and point the workflow
--    at it. Idempotent — `ON CONFLICT DO NOTHING` covers re-runs and the
--    UPDATE skips workflows that already have an active version.
DO $$
DECLARE
    w           RECORD;
    new_version TEXT;
    snapshot    JSONB;
BEGIN
    FOR w IN
        SELECT workflow_id, name, trigger_kind, trigger_config, created_by,
               created_at, workspace_id, install_id
          FROM workflows
         WHERE active_version_id IS NULL
    LOOP
        new_version := 'wfv_' || w.workflow_id;

        SELECT jsonb_build_object(
            'id',           w.workflow_id,
            'name',         w.name,
            'workspace_id', w.workspace_id,
            'install_id',   w.install_id,
            'trigger',      jsonb_build_object(
                                'kind',   w.trigger_kind,
                                'config', w.trigger_config
                            ),
            'stages',       COALESCE(
                                (SELECT jsonb_agg(
                                            jsonb_build_object(
                                                'stage_order',  s.stage_order,
                                                'name',         s.name,
                                                'target_state', s.target_state,
                                                'gates',        s.gates,
                                                'params',       s.params
                                            )
                                            ORDER BY s.stage_order
                                        )
                                   FROM workflow_stages s
                                  WHERE s.workflow_id = w.workflow_id),
                                '[]'::jsonb
                            )
        ) INTO snapshot;

        INSERT INTO workflow_versions (
            version_id, workflow_id, version_label, content,
            parent_version_id, state, created_by, created_at, published_at
        ) VALUES (
            new_version, w.workflow_id, 'v1', snapshot,
            NULL, 'published', w.created_by, w.created_at, w.created_at
        )
        ON CONFLICT (version_id) DO NOTHING;

        UPDATE workflows
           SET active_version_id = new_version
         WHERE workflow_id = w.workflow_id
           AND active_version_id IS NULL;

        INSERT INTO workflow_changes (
            workflow_id, version_id, actor, action,
            before_content, after_content, reason, created_at
        ) VALUES (
            w.workflow_id, new_version, w.created_by, 'publish',
            NULL, snapshot, 'migration 025 v1 backfill', w.created_at
        );
    END LOOP;
END $$;
