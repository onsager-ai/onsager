-- Issue #156: workflows.created_by — owner identity for credential lookup.
--
-- Forge populates `ShapingRequest.created_by` from this column so stiglab
-- can decrypt the right user's `CLAUDE_CODE_OAUTH_TOKEN` when dispatching
-- the agent. Without it every workflow-dispatched session boots without
-- OAuth and exits immediately ("stdout closed without result event").
--
-- Nullable on purpose: existing rows pre-date this migration and stay
-- NULL per the no-backfill decision. Workflows with NULL created_by fail
-- loudly on next dispatch via `stiglab.session_failed` until the owner
-- re-activates and the activation hook re-mirrors the row with their
-- user_id. New activations enforce that the caller has ≥1 credential
-- row, see `workflows.rs::patch_workflow`.

ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS created_by TEXT;
