-- Onsager #499 / #500 (RUN-01 #359, RUN-03 #386) — durable substrate
-- scheduler state.
--
-- RUN-01 specced the substrate scheduler as "spine-persisted,
-- restart-safe" but the shipped run path used an in-memory PlanStore.
-- These tables back the durable `PlanStore` so node states and
-- materialized artifacts survive a host restart, and a small plan
-- registry lets the host reload + recompile any plan with non-terminal
-- nodes and re-drive it to completion (ADR 0023, decision 1).
--
-- The scheduler host also ensures these tables idempotently at startup
-- (crates/onsager-scheduler/src/migrate.rs) so a fresh deploy works
-- whichever process migrates first — same `CREATE TABLE IF NOT EXISTS`
-- shape, the loser is a no-op.
--
-- Pre-launch posture (CLAUDE.md): no backfill, no FK constraints
-- (matches the rest of the spine schema — joins happen at the app
-- layer).

-- One row per Execution Plan run. `spec_plan_json` is the compiled-from
-- source so a restart can recompile against the current library and
-- resume; `spec_plan_id` is the originating persisted plan id when the
-- run came from `run_spec_plan` (NULL for the single-spec trigger.fired
-- path, whose source is synthesized).
CREATE TABLE IF NOT EXISTS execution_plans (
    plan_id         TEXT        NOT NULL PRIMARY KEY,
    workspace_id    TEXT        NOT NULL DEFAULT 'default',
    spec_plan_id    TEXT,
    spec_plan_json  JSONB       NOT NULL,
    status          TEXT        NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'completed', 'failed')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Recovery scan: "plans that are still running".
CREATE INDEX IF NOT EXISTS idx_execution_plans_status
    ON execution_plans (status)
    WHERE status = 'running';

-- Per-node lifecycle state, one row per (plan, node). The scheduler
-- writes a transition before and after dispatch; recovery reads this
-- map and resets non-terminal (ready/running) states to pending.
CREATE TABLE IF NOT EXISTS execution_plan_nodes (
    plan_id     TEXT        NOT NULL,
    node_id     TEXT        NOT NULL,
    state       TEXT        NOT NULL
        CHECK (state IN ('pending', 'ready', 'running', 'completed', 'failed')),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (plan_id, node_id)
);

-- Materialized artifacts keyed by (plan, artifact_id). Single-writer
-- per ArtifactId is enforced at compile time (invariant 5), so the
-- upsert is the same write on a re-dispatch, not contention.
CREATE TABLE IF NOT EXISTS execution_plan_artifacts (
    plan_id       TEXT        NOT NULL,
    artifact_id   TEXT        NOT NULL,
    artifact_json JSONB       NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (plan_id, artifact_id)
);
