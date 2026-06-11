-- Onsager #582 (ADR 0027) — the synodic PR gate retired with the
-- governance subsystem; nothing writes or reads pr_gate_verdicts.
DROP TABLE IF EXISTS pr_gate_verdicts;
