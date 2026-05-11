-- Onsager #312: portal-owned cost ledger for `propose_remediation`'s
-- server-side AI call.
--
-- One row per AI call. Used for:
--   1. Per-workspace monthly budget enforcement (sum cost_usd over the
--      current calendar month; refuse new calls past the cap).
--   2. Observability — `SELECT workspace_id, SUM(cost_usd) ... GROUP BY`
--      tells operators which workspaces are paying for remediation.
--
-- Schema is intentionally narrow: token counts so we can recompute cost
-- if the upstream price changes, plus a denormalized `cost_usd` snapshot
-- captured at call time using the price table compiled into the binary.
-- No FK on workspace_id — same pattern as the rest of portal's tables,
-- which join back to spine.workspaces at the app layer.

CREATE TABLE IF NOT EXISTS portal_remediation_calls (
    id                          TEXT        PRIMARY KEY,
    workspace_id                TEXT        NOT NULL,
    user_id                     TEXT        NOT NULL,
    artifact_id                 TEXT        NOT NULL,
    model                       TEXT        NOT NULL,
    input_tokens                INTEGER     NOT NULL DEFAULT 0,
    output_tokens               INTEGER     NOT NULL DEFAULT 0,
    cache_creation_input_tokens INTEGER     NOT NULL DEFAULT 0,
    cache_read_input_tokens     INTEGER     NOT NULL DEFAULT 0,
    cost_usd                    DOUBLE PRECISION NOT NULL DEFAULT 0,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_portal_remediation_calls_workspace_month
    ON portal_remediation_calls (workspace_id, created_at DESC);
