-- Portal-owned schema: per-user, per-workspace encrypted credentials
-- (e.g. CLAUDE_CODE_OAUTH_TOKEN, ANTHROPIC_API_KEY).
--
-- Spec #222 Slice 2a — moved from stiglab's inline `CREATE TABLE` block
-- to portal. Portal owns the read/write surface via
-- `/api/workspaces/:id/credentials*`; stiglab still reads encrypted
-- values from its `AnyPool` when launching agent sessions (the
-- credentials are decrypted in-process and handed to the agent as env
-- vars). Same database, separate connection pool; portal is the only
-- writer.
--
-- `encrypted_value` is the hex-encoded AES-256-GCM nonce + ciphertext
-- produced by `encrypt_credential` (see `onsager_portal::auth`).
-- `workspace_id` is the workspace the credential is scoped to —
-- credentials are workspace-isolated since #164/#161 child C, so a
-- session in W1 can never reach a token registered in W2.
--
-- `workspace_id` is intentionally TEXT without a foreign key to
-- `workspaces` for the same reason `portal_webhook_secrets` and
-- `user_pats` skip the FK: the `workspaces` table currently lives in
-- stiglab's runtime migrations (target: spine, per spec #222 Slice 3).
-- Once workspaces move into the spine, the FK can be added in a
-- follow-up migration.

CREATE TABLE IF NOT EXISTS user_credentials (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    name            TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (workspace_id, user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_user_credentials_workspace_user_name
    ON user_credentials (workspace_id, user_id, name);

CREATE INDEX IF NOT EXISTS idx_user_credentials_workspace
    ON user_credentials (workspace_id);
