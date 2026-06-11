-- Onsager #584 (ADR 0027) — the observer tier retired (ising +
-- onsager-observers deleted); nothing writes or reads observer_outputs.
-- The CREATE was removed from 005_substrate.sql for fresh installs;
-- this drop cleans up shared environments that already have the table.
DROP TABLE IF EXISTS observer_outputs;
