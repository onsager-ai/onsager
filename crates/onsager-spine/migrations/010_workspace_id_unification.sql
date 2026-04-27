-- Onsager #131 Lever D: spine tables become the single source of truth for
-- workspace scope. Spec issue #162 (parent #161).
--
-- Migration 004 added `workspace_id NOT NULL DEFAULT 'default'` to the
-- artifact-pipeline registry tables (artifacts, artifact_types,
-- gate_evaluators, agent_profiles, artifact_adapters). It also added
-- `workspace_id TEXT NOT NULL` to `registry_seed_marker` without a default.
-- The workflow + warehouse + delivery tables shipped before that decision
-- and were missed; this migration brings them onto the same shape so list
-- endpoints can filter by workspace and the parent spec's contract test
-- can pass.
--
-- workflow_stages is intentionally NOT augmented: it's keyed by
-- (workflow_id, stage_order) and inherits scope through the FK to
-- workflows(workflow_id) ON DELETE CASCADE.
--
-- workflows.workspace_install_ref is intentionally retained, NOT dropped.
-- Despite its name, the column does not encode workspace scope; it is the
-- GitHub install id (TEXT, populated by stiglab's mirror at
-- crates/stiglab/src/server/workflow_spine_mirror.rs:33 via
-- `workflow.install_id.to_string()`) and read back into the typed
-- `Workflow.workspace_install_ref` field at
-- crates/forge/src/core/workflow_persistence.rs:68. Both ends are live
-- production wires, so per CLAUDE.md "no dangling wires" — the wire is
-- *connected*, not loose. Renaming it to `install_id` is a separate spec
-- (parent #161, sibling track) so this migration leaves the column as-is.

-- Workflows: workspace scope for listing/filtering.
ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_workflows_workspace ON workflows (workspace_id);

-- Bundles: warehouse-sealed snapshots inherit workspace scope from their
-- artifact, but carry the column directly so list queries don't need a
-- join through artifacts.
ALTER TABLE bundles
    ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_bundles_workspace ON bundles (workspace_id);

-- Consumer sinks: per-artifact delivery destinations. Same rationale as
-- bundles — column on the row keeps list filters cheap.
ALTER TABLE consumer_sinks
    ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_consumer_sinks_workspace
    ON consumer_sinks (workspace_id);

-- Deliveries: per (bundle, consumer) attempts. Column is mirrored here so
-- workspace-scoped delivery dashboards don't have to join through bundles
-- or consumer_sinks on every page load.
ALTER TABLE deliveries
    ADD COLUMN IF NOT EXISTS workspace_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_deliveries_workspace ON deliveries (workspace_id);
