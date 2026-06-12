-- 0.2 substrate extensions: reconciliation state.
--
-- adapter_reconciliation_state — per-(adapter, workspace, resource_kind)
--   poll cursors and ETag cache. Webhooks miss deliveries (GitHub's docs are
--   explicit); the reconciliation poller is the backstop. Idempotency between
--   webhook and poll paths is enforced by the partial unique index on
--   events_ext (spec #121).


-- Per-(adapter, workspace, resource_kind) reconciliation cursors.
-- last_seen_external_id / last_seen_updated_at are the high-water mark;
-- etag supports conditional requests (If-None-Match) so a 304 from GitHub
-- skips work and doesn't count against the rate limit.
CREATE TABLE IF NOT EXISTS adapter_reconciliation_state (
    adapter_id              TEXT        NOT NULL,
    workspace_id            TEXT        NOT NULL,
    resource_kind           TEXT        NOT NULL,
    last_seen_external_id   TEXT,
    last_seen_updated_at    TIMESTAMPTZ,
    etag                    TEXT,
    last_polled_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (adapter_id, workspace_id, resource_kind)
);

CREATE INDEX IF NOT EXISTS idx_adapter_reconciliation_state_workspace
    ON adapter_reconciliation_state (workspace_id);
