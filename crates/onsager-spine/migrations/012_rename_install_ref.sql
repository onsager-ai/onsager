-- Onsager #131 Lever D (#149), phase 1: column rename only.
--
-- The mislabelled `workflows.workspace_install_ref` column has always
-- stored the GitHub install id (TEXT), not workspace scope. Migration 010
-- explicitly retained it because the mirror module
-- (`crates/stiglab/src/server/workflow_spine_mirror.rs`) was still its only
-- writer; renaming the column was deferred to Lever D.
--
-- This phase ships the rename atomically across the migration, the forge
-- reader (`crates/forge/src/core/workflow_persistence.rs`), the typed
-- `Workflow.install_id` field, and the mirror writer. The actual table
-- collapse — folding stiglab `workspace_workflows` into spine `workflows`
-- and deleting the mirror module — is the rest of Lever D and lands as a
-- follow-up; it requires a stiglab `workflow_db.rs` rewrite plus a
-- Postgres-backed test harness that's bigger than this PR.

ALTER TABLE workflows RENAME COLUMN workspace_install_ref TO install_id;
