-- v2 schema, migration 004 — normalized agent session events (#632).
--
-- One row per normalized `agent.*` event captured from a session's
-- stream-json output (see `agent_events.rs` for the vocabulary).
-- Consumed by `GET /api/sessions/:id/events` and the run page's
-- session feed — landing together, per the ratchet.

CREATE TABLE session_events (
    id         BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq        INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    payload    JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, seq)
);
CREATE INDEX idx_session_events_session ON session_events (session_id, seq);
