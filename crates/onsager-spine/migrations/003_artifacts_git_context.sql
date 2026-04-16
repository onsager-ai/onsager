-- Onsager Level 2.1: Git context for pull request lifecycle metadata.
-- Stores repo/branch/PR linkage for PR artifacts and related work.
ALTER TABLE artifacts
ADD COLUMN IF NOT EXISTS git_context JSONB;
