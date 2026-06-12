-- Onsager #603 (ADR 0028) — registry diet. The 0.1 DB-backed registry
-- (types / evaluators / profiles / adapters as data) never gained a
-- reader: the workflow-kind catalog is code-defined and the gate
-- machinery retired with ADR 0027. Drops the migration-003 tables in
-- shared envs; the 003 file is now a numbered no-op stub.
DROP TABLE IF EXISTS registry_seed_marker;
DROP TABLE IF EXISTS artifact_adapters;
DROP TABLE IF EXISTS agent_profiles;
DROP TABLE IF EXISTS gate_evaluators;
DROP TABLE IF EXISTS artifact_types;
