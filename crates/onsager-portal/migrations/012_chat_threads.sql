-- Portal-owned schema: server-side chat storage with cross-scope history
-- index (spec #470, implements ADR 0020).
--
-- Chat conversations are user-private state, not factory-coordination
-- state — they don't belong on the spine event bus. The schema lives in
-- portal-owned tables alongside the spine in the same Postgres database;
-- portal is the only reader and writer.
--
-- Schema shape is fixed by ADR 0020:
--   (user_id, scope_type, scope_id) → one rolling thread
-- with scope_type in (workspace, workflow, run, verdict). The
-- reserved-not-required `thread_id` axis is satisfied by
-- `chat_threads.id` — a single rolling thread per scope today; multi-
-- thread per scope can land later without migration.
--
-- `workspace_id` is intentionally TEXT without a foreign key to
-- `workspaces` for the same reason the other portal-owned tables skip
-- it: joins to `spine.workspaces` happen at the app layer.

CREATE TABLE IF NOT EXISTS chat_threads (
    id           TEXT         NOT NULL PRIMARY KEY,
    user_id      TEXT         NOT NULL,
    workspace_id TEXT         NOT NULL,
    scope_type   TEXT         NOT NULL
        CHECK (scope_type IN ('workspace', 'workflow', 'run', 'verdict')),
    -- NULL for scope_type='workspace'; the externally-meaningful id
    -- otherwise. The `scope_id_key` generated column below collapses
    -- NULLs to '' so the unique constraint works for the workspace-
    -- scope case (Postgres treats NULL as distinct in UNIQUE).
    scope_id     TEXT,
    scope_id_key TEXT         GENERATED ALWAYS AS (COALESCE(scope_id, '')) STORED,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, workspace_id, scope_type, scope_id_key)
);

CREATE INDEX IF NOT EXISTS idx_chat_threads_workspace_user
    ON chat_threads (workspace_id, user_id);

CREATE TABLE IF NOT EXISTS chat_messages (
    id           TEXT         NOT NULL PRIMARY KEY,
    thread_id    TEXT         NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    role         TEXT         NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content      TEXT         NOT NULL,
    tool_call_id TEXT,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    -- Full-text-search column for cross-scope search. Auto-maintained
    -- via a GENERATED column instead of a trigger — same semantics, less
    -- machinery. The spec's plan item asks for a trigger that keeps the
    -- column updated; the GENERATED expression satisfies that
    -- requirement (Postgres re-computes on every INSERT/UPDATE).
    content_tsv  TSVECTOR     GENERATED ALWAYS AS (to_tsvector('english', content)) STORED
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_thread_id
    ON chat_messages (thread_id, created_at);

CREATE INDEX IF NOT EXISTS idx_chat_messages_content_tsv
    ON chat_messages USING GIN (content_tsv);
