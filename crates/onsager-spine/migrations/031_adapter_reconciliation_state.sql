-- Onsager #121 — Adapter reconciliation state (poll cursors per adapter
-- × workspace × resource kind).
--
-- Webhooks miss deliveries: GitHub's docs are explicit that delivery is
-- best-effort. The poll-based reconciliation contract (#121) pairs the
-- webhook path with a periodic adapter-level poll that catches anything
-- the webhook dropped. Each (adapter, workspace, resource_kind) tuple
-- carries a high-water mark plus an optional ETag for conditional
-- requests so a quiet repo is effectively free against the rate limit.
--
-- See spec #121 § Design / "adapter_reconciliation_state table".

CREATE TABLE IF NOT EXISTS adapter_reconciliation_state (
    adapter_id           TEXT NOT NULL,
    workspace_id         TEXT NOT NULL,
    resource_kind        TEXT NOT NULL,
    -- The most-recent external resource id (e.g. issue number, PR
    -- number) the adapter has surfaced to the spine. Optional because
    -- a fresh state row has not yet observed anything.
    last_seen_external_id   TEXT,
    -- Mirrors GitHub's `updated_at`; used as the `since` cursor on
    -- subsequent polls so we only ask for resources that changed
    -- after the last successful merge.
    last_seen_updated_at    TIMESTAMPTZ,
    -- ETag from the most recent successful poll. When present, the
    -- next request sends `If-None-Match: <etag>` so a 304 skips work
    -- and does not count against the authenticated rate limit.
    etag                    TEXT,
    -- Wall-clock timestamp of the most recent poll attempt (success
    -- OR 304 OR error). Drives jitter/backoff at the scheduler tick.
    last_polled_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (adapter_id, workspace_id, resource_kind)
);

CREATE INDEX IF NOT EXISTS idx_adapter_reconciliation_state_workspace
    ON adapter_reconciliation_state (workspace_id);
