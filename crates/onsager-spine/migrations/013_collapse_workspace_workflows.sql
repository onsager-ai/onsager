-- Onsager #131 Lever D (#149), phase 2: collapse stiglab workspace_workflows
-- into spine workflows.
--
-- Migration 010 added `workspace_id` to spine `workflows`.
-- Migration 012 (#219) renamed `workspace_install_ref` → `install_id`.
-- This migration completes Lever D by:
--   1. Backfilling spine `workflows` / `workflow_stages` from any rows still
--      living in stiglab's `workspace_workflows` / `workspace_workflow_stages`
--      tables (when those tables exist in the same database — the production
--      topology in `.env.example`).
--   2. Dropping `workspace_workflows` / `workspace_workflow_stages` so stiglab
--      can write to the spine schema directly without a translator module.
--
-- The whole migration is wrapped in a `DO $$` block guarded on the source
-- tables existing. Topologies where stiglab and spine point at separate
-- databases simply skip everything — stiglab's bootstrap no longer creates
-- the source tables either, so a fresh deploy lands on the spine schema
-- with nothing to migrate.
--
-- Idempotent on purpose: same-shape guard makes second-and-later boots a
-- no-op, matching the rest of the migrations directory.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM information_schema.tables
         WHERE table_schema = 'public'
           AND table_name = 'workspace_workflows'
    ) THEN
        -- 1a. Workflows. The mirror module wrote the same translation we
        --     do here on every CRUD call, so most production rows already
        --     exist in spine.workflows; the ON CONFLICT clause makes the
        --     fill-in safe for the long tail that didn't make it through.
        --
        --     trigger_kind: stiglab persists 'github-issue-webhook' (kebab);
        --     spine's CHECK constraint requires 'github_issue_webhook' (snake).
        --     trigger_config: packed from per-row repo + label columns into
        --     the JSON shape forge already reads.
        INSERT INTO workflows (
            workflow_id, name, trigger_kind, trigger_config,
            active, preset_id, workspace_id, install_id, created_by,
            created_at, updated_at
        )
        SELECT  ww.id,
                ww.name,
                CASE ww.trigger_kind
                    WHEN 'github-issue-webhook' THEN 'github_issue_webhook'
                    ELSE ww.trigger_kind
                END,
                jsonb_build_object(
                    'repo',  ww.repo_owner || '/' || ww.repo_name,
                    'label', ww.trigger_label
                ),
                (ww.active <> 0),
                ww.preset_id,
                ww.workspace_id,
                ww.install_id::text,
                ww.created_by,
                COALESCE(ww.created_at::timestamptz, NOW()),
                COALESCE(ww.updated_at::timestamptz, NOW())
          FROM  workspace_workflows ww
        ON CONFLICT (workflow_id) DO NOTHING;

        -- 1b. Workflow stages. spine.workflow_stages is keyed by
        --     (workflow_id, stage_order); stiglab stored an opaque `id` and
        --     a `seq` int. The mirror translated gate_kind + params into
        --     (target_state, gates) for forge's stage runner; we lift the
        --     same translation into SQL CASE expressions here.
        INSERT INTO workflow_stages (
            workflow_id, stage_order, name, target_state, gates, params
        )
        SELECT  s.workflow_id,
                s.seq,
                s.gate_kind,
                CASE s.gate_kind
                    WHEN 'agent-session'    THEN 'in_progress'
                    WHEN 'external-check'   THEN 'under_review'
                    WHEN 'governance'       THEN 'under_review'
                    WHEN 'manual-approval'  THEN 'under_review'
                    ELSE NULL
                END,
                CASE s.gate_kind
                    WHEN 'agent-session' THEN
                        jsonb_build_array(jsonb_build_object(
                            'kind', 'agent_session',
                            'shaping_intent', s.params::jsonb
                        ))
                    WHEN 'external-check' THEN
                        jsonb_build_array(jsonb_build_object(
                            'kind', 'external_check',
                            'check_name', COALESCE(
                                (s.params::jsonb) ->> 'check_name', 'ci'
                            )
                        ))
                    WHEN 'governance' THEN
                        jsonb_build_array(
                            CASE
                                WHEN (s.params::jsonb) ? 'gate_point' THEN
                                    jsonb_build_object(
                                        'kind', 'governance',
                                        'gate_point',
                                        (s.params::jsonb) ->> 'gate_point'
                                    )
                                ELSE
                                    jsonb_build_object('kind', 'governance')
                            END
                        )
                    WHEN 'manual-approval' THEN
                        jsonb_build_array(jsonb_build_object(
                            'kind', 'manual_approval',
                            'signal_kind', COALESCE(
                                (s.params::jsonb) ->> 'signal_kind',
                                'dashboard_approve'
                            )
                        ))
                    ELSE '[]'::jsonb
                END,
                s.params::jsonb
          FROM  workspace_workflow_stages s
        ON CONFLICT (workflow_id, stage_order) DO NOTHING;

        -- 2. Drop the source tables. Stiglab's writer already targets
        --    spine.workflows / spine.workflow_stages from this PR onwards;
        --    nothing else still depends on these names.
        DROP TABLE workspace_workflow_stages;
        DROP TABLE workspace_workflows;
    END IF;
END $$;
