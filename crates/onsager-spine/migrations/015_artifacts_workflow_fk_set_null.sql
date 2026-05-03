-- Onsager #233 — workflow delete blocked by artifact FK.
--
-- Migration 006 added `artifacts.workflow_id ... REFERENCES
-- workflows(workflow_id)` with no `ON DELETE` clause, so the FK
-- defaults to NO ACTION. The sibling FK on `workflow_stages` declared
-- ON DELETE CASCADE in the same migration — asymmetric behavior on
-- two columns of the same name pointing at the same parent. Result:
-- `DELETE FROM workflows WHERE workflow_id = $1` raised whenever any
-- artifact still pointed at the row, even if the workflow was already
-- deactivated, leaving the dashboard's delete button broken.
--
-- Switch to ON DELETE SET NULL: workflow lifecycle no longer cascades
-- through artifacts (which represent real work product), but the
-- workflow row can be deleted without leaving the FK in a violated
-- state. Symmetric with `workflow_stages` in spirit (parent goes,
-- child relinquishes its tag) without the data-destroying cascade.
--
-- The application-side `workflow_db::delete_workflow` (#233 PR) wraps
-- the delete in a transaction that also NULLs `current_stage_index`
-- and `workflow_parked_reason` on the orphaned artifacts in the same
-- step, so the read side ("not workflow-tagged" iff
-- workflow_id IS NULL) stays consistent. This migration is the
-- defense-in-depth half: any future code path that DELETEs from
-- `workflows` directly still won't violate the FK, even if it forgets
-- the application cleanup.

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_workflow_id_fkey;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_workflow_id_fkey
    FOREIGN KEY (workflow_id)
    REFERENCES workflows(workflow_id)
    ON DELETE SET NULL;
