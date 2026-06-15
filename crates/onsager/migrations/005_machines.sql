-- v2 schema, migration 005 — fleet machine registry (ADR 0030 M4).
--
-- Durable record of worker machines that have dialed in: a connection
-- upserts the row online, a disconnect/lease-expiry marks it offline.
-- Consumed by `GET /api/fleet/machines` and the dashboard Machines
-- (operator) surface — landing together, per the ratchet.

CREATE TABLE machines (
    name         TEXT PRIMARY KEY,
    status       TEXT NOT NULL DEFAULT 'offline',
    connected_at TIMESTAMPTZ,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
