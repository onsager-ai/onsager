-- Portal-owned schema: server-issued bearer tokens (Personal Access Tokens
-- per issue #143) that authenticate non-browser callers as a specific user.
--
-- Spec #222 Slice 2b — moved from stiglab's inline `CREATE TABLE` block to
-- portal. Portal mints, lists, and revokes PATs through `/api/pats*`; the
-- AuthUser extractor on portal verifies presented tokens. Stiglab still
-- reads this table from its `AnyPool` for its own `AuthUser` extractor —
-- credentials/workspaces/projects/workflows still live behind the seam on
-- stiglab and accept PAT bearer auth — until Slices 2a, 3, and 4 finish
-- moving those routes. Same database, separate connection pool; portal is
-- the only writer.
--
-- The full token is never stored: `token_hash` is the SHA-256 hex of the
-- raw token, `token_prefix` is the first `PAT_PREFIX_LEN` characters for
-- display + indexed lookup. `workspace_id` is the workspace this PAT is
-- pinned to (mandatory since #163 — cross-workspace calls 403). `revoked_at`
-- is a soft-delete: revoked rows stay for audit but fail verification.
--
-- `workspace_id` is intentionally TEXT without a foreign key to `workspaces`
-- for the same reason `portal_webhook_secrets` skips the FK: the
-- `workspaces` table currently lives in stiglab's runtime migrations
-- (target: spine, per spec #222 Slice 3). Once workspaces move into the
-- spine, the FK can be added in a follow-up migration.

CREATE TABLE IF NOT EXISTS user_pats (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    workspace_id         TEXT NOT NULL,
    name                 TEXT NOT NULL,
    token_prefix         TEXT NOT NULL,
    token_hash           TEXT NOT NULL,
    scopes               TEXT NOT NULL DEFAULT '["*"]',
    expires_at           TEXT,
    last_used_at         TEXT,
    last_used_ip         TEXT,
    last_used_user_agent TEXT,
    created_at           TEXT NOT NULL,
    revoked_at           TEXT,
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_user_pats_user_id      ON user_pats (user_id);
CREATE INDEX IF NOT EXISTS idx_user_pats_token_prefix ON user_pats (token_prefix);
