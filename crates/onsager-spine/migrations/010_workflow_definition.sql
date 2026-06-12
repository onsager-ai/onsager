-- Onsager #602 (ADR 0028) — one workflow model.
--
-- The authored stage list moves onto the workflows row
-- (definition_json); the executable form lives in workflow_library
-- keyed by the workflow id. workflow_stages — the legacy Lever D
-- stage-chain storage the retired forge gate machine consumed — drops.

ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS definition_json JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Backfill the authored form from the legacy stage rows (gate kind +
-- params, ordered). Idempotent: only fills rows still at the default.
UPDATE workflows w
   SET definition_json = sub.def
  FROM (
        SELECT workflow_id,
               jsonb_agg(
                   jsonb_build_object('gate_kind', name, 'params', params)
                   ORDER BY stage_order
               ) AS def
          FROM workflow_stages
      GROUP BY workflow_id
       ) sub
 WHERE sub.workflow_id = w.workflow_id
   AND w.definition_json = '[]'::jsonb;

DROP TABLE IF EXISTS workflow_stages;
