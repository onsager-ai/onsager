-- Portal-owned schema: server-side cookie sessions minted by portal's
-- OAuth callback / dev-login / SSO-finish routes. The `stiglab_session`
-- cookie value is the row PK; `expires_at` is checked on every read.
--
-- Spec #222 Slice 5. Stiglab still reads this table on every authenticated
-- request via its `AuthUser` extractor (cookie path); portal is the writer.

CREATE TABLE IF NOT EXISTS auth_sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user_id ON auth_sessions (user_id);
