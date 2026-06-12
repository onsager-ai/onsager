-- Onsager #395 — 0.2 substrate authoring tools via MCP.
--
-- Persistence for `SpecPlan` (ADR 0015) so the MCP authoring surface
-- can store author-written plans and recall them on read. The schema
-- mirrors the on-the-wire `SpecPlan` struct: a JSONB column for the
-- specs + deps arrays plus the audit columns the other portal-owned
-- tables use.
--
-- Keyed on `(workspace_id, spec_plan_id)` — `SpecId`s within a plan
-- are externally-assigned per ADR 0015 (GitHub issue number,
-- `mcp:<uuid>`, etc.); the plan-level id is the row identity.
-- `created_by` carries the authoring user id (audit trail; reuses
-- the same shape `factory_tasks` / `workflows` use).
--
-- Pre-launch posture (CLAUDE.md): no backfill, no soft-delete column
-- yet, no FK on workspace_id (matches the rest of portal — joins to
-- spine.workspaces happen at the app layer).

CREATE TABLE IF NOT EXISTS spec_plans (
    workspace_id    TEXT        NOT NULL,
    spec_plan_id    TEXT        NOT NULL,
    plan_json       JSONB       NOT NULL,
    created_by      TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, spec_plan_id)
);

CREATE INDEX IF NOT EXISTS idx_spec_plans_workspace_created
    ON spec_plans (workspace_id, created_at DESC);
