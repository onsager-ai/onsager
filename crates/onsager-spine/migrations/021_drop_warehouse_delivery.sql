-- Spec #274: remove the warehouse / bundle / delivery skeleton.
--
-- The original v0.1 spec (#20) shipped Bundle / Delivery / Consumer types
-- with one in-tree user (forge sealing on Released) and zero downstream
-- consumers. None of the deferred slices ever landed; the spec #274 cut
-- removes the now-dead Rust types, events, and tables. This migration
-- mirrors that on the database side.

-- Drop the FK from artifacts → bundles before either column or table
-- can go away. Constraint name comes from migration 007's rename.
ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_current_version_fkey;

-- Drop the now-dead artifact-side warehouse pointer columns.
ALTER TABLE artifacts
    DROP COLUMN IF EXISTS current_version_id,
    DROP COLUMN IF EXISTS version_history;

-- Order respects FKs: deliveries → bundles → consumer_sinks → bundles.
DROP TABLE IF EXISTS deliveries;
DROP TABLE IF EXISTS consumer_sinks;
DROP TABLE IF EXISTS bundles;
