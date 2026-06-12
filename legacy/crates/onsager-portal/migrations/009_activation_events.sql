-- Portal-owned schema: FTUE activation funnel events (spec #404).
--
-- The umbrella (#397) defines a four-rung activation ladder
-- (Inspected → Drafted → Bound → Activated). This table is the
-- append-only sink every rung writes to. Portal — not spine. Activation
-- events are dashboard-telemetry semantics, not factory-coordination
-- semantics; they don't belong on the event bus (per the seam rule,
-- factory coordination ≠ user-behavior telemetry).
--
-- Fire-once-per-(user, draft|workflow) is enforced server-side via the
-- UNIQUE constraint on `dedup_key`. The handler / listener compute the
-- key from the closed event-name + context shape:
--
--   ftue.inspected: ftue.inspected|{user_id|anonymous_id}
--   ftue.drafted  : ftue.drafted|{user_id|anonymous_id}|{draft_id}
--   ftue.bound    : ftue.bound|{user_id}|{draft_id}
--   ftue.activated: ftue.activated|{user_id}|{workflow_id}
--
-- `user_id` is intentionally TEXT without a FK to `users` for the same
-- reason `portal_webhook_secrets` and `user_pats` skip the FK: keeps the
-- portal migrations self-contained and lets pre-auth `ftue.inspected`
-- rows (NULL user_id) coexist with authenticated rows.

CREATE TABLE IF NOT EXISTS activation_events (
    id           TEXT        PRIMARY KEY,
    event        TEXT        NOT NULL,
    occurred_at  TIMESTAMPTZ NOT NULL,
    user_id      TEXT,
    anonymous_id TEXT        NOT NULL,
    surface      TEXT        NOT NULL,
    path         TEXT        NOT NULL,
    context      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    dedup_key    TEXT        NOT NULL UNIQUE,
    received_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Funnel reporting walks (event, user_id || anonymous_id) over an
-- occurred_at range. The unique dedup_key index doesn't help that
-- query; this one does.
CREATE INDEX IF NOT EXISTS idx_activation_events_event_occurred
    ON activation_events (event, occurred_at DESC);
