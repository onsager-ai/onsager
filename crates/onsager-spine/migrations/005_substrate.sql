-- 0.2 substrate extensions: observer outputs and reconciliation state.
--
-- observer_outputs — append-only sink for observer-emitted analysis
--   (QualitySignal / Insight / Alert). Observers subscribe to spine events,
--   run analysis off the hot path, and write here; they never write to
--   events / events_ext (spec #361 / ADR 0013).
--
-- adapter_reconciliation_state — per-(adapter, workspace, resource_kind)
--   poll cursors and ETag cache. Webhooks miss deliveries (GitHub's docs are
--   explicit); the reconciliation poller is the backstop. Idempotency between
--   webhook and poll paths is enforced by the partial unique index on
--   events_ext (spec #121).

CREATE TABLE IF NOT EXISTS observer_outputs (
    id                    BIGSERIAL   NOT NULL PRIMARY KEY,
    observer_id           TEXT        NOT NULL,
    kind                  TEXT        NOT NULL
        CHECK (kind IN ('quality_signal', 'insight', 'alert')),
    triggered_by_event_id BIGINT
        REFERENCES events(id) ON DELETE SET NULL,
    payload               JSONB       NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Dashboard reads: "latest outputs from observer X" and "latest alerts".
CREATE INDEX IF NOT EXISTS observer_outputs_observer_id_id_idx
    ON observer_outputs (observer_id, id DESC);
CREATE INDEX IF NOT EXISTS observer_outputs_kind_id_idx
    ON observer_outputs (kind, id DESC);
-- Drill-down: "what did observers say about event #X".
CREATE INDEX IF NOT EXISTS observer_outputs_triggered_by_event_id_idx
    ON observer_outputs (triggered_by_event_id)
    WHERE triggered_by_event_id IS NOT NULL;

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
