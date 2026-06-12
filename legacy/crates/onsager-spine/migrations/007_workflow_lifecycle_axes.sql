-- Workflow lifecycle axes (ADR 0024, spec #506 Phase 1).
--
-- Replace the `(active, install_id)`-derived workflow status with two
-- explicit, orthogonal, trigger-source-neutral columns. `install_id` (a
-- GitHub App installation id) stops defining lifecycle status; it stays
-- on the row only as a token-mint cache.
--
--   readiness : drafted → ready   (ready ⇔ the workflow has a usable
--                                   trigger binding of any kind)
--   autofire  : off / active / paused
--
-- The two axes are independent: a `ready` workflow may be `off`
-- (manual-only), `active` (auto-firing on its trigger), or `paused`.
--
-- `active` (the legacy boolean firing pin read by the runtime) is kept
-- and held in sync with `autofire` by portal's single writer path
-- (`insert_workflow_with_stages` + `set_workflow_active`). Collapsing
-- `active` into `autofire` as the sole source of truth is a tracked
-- follow-up under #506 (it touches the runtime's `active = TRUE` read).

ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS readiness TEXT NOT NULL DEFAULT 'ready'
        CHECK (readiness IN ('drafted', 'ready'));

ALTER TABLE workflows
    ADD COLUMN IF NOT EXISTS autofire TEXT NOT NULL DEFAULT 'off'
        CHECK (autofire IN ('off', 'active', 'paused'));

-- Backfill: an active workflow auto-fires. Idempotent (re-runs are
-- no-ops once promoted) and never clobbers a `paused` row, since no
-- paused rows exist at backfill time. `readiness` defaults to `ready`
-- for every existing row — they all carry a trigger binding today.
UPDATE workflows SET autofire = 'active' WHERE active = TRUE AND autofire = 'off';

CREATE INDEX IF NOT EXISTS idx_workflows_readiness ON workflows (readiness);
CREATE INDEX IF NOT EXISTS idx_workflows_autofire  ON workflows (autofire);
