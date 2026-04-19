use std::time::Duration;

use crate::core::{
    GitHubAppInstallation, Node, NodeStatus, Project, Session, SessionState, Tenant, TenantMember,
    User,
};
use chrono::Utc;
use sqlx::pool::PoolOptions;
use sqlx::AnyPool;

pub async fn init_pool(database_url: &str) -> anyhow::Result<AnyPool> {
    // For SQLite: ensure parent directory exists and enable create-if-missing
    let connect_url = if database_url.starts_with("sqlite://") {
        let path = database_url.trim_start_matches("sqlite://");
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        // Append mode=rwc so SQLx creates the file if it doesn't exist
        if database_url.contains('?') {
            format!("{database_url}&mode=rwc")
        } else {
            format!("{database_url}?mode=rwc")
        }
    } else {
        database_url.to_string()
    };

    // Install drivers
    sqlx::any::install_default_drivers();

    let pool = tokio::time::timeout(
        Duration::from_secs(10),
        PoolOptions::new()
            .acquire_timeout(Duration::from_secs(10))
            .connect(&connect_url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out while connecting to database"))??;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &AnyPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS nodes (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            hostname TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'online',
            max_sessions INTEGER NOT NULL DEFAULT 4,
            active_sessions INTEGER NOT NULL DEFAULT 0,
            last_heartbeat TEXT NOT NULL,
            registered_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending',
            prompt TEXT NOT NULL,
            output TEXT,
            working_dir TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS session_logs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            chunk TEXT NOT NULL,
            stream TEXT NOT NULL DEFAULT 'stdout',
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_session_logs_session_id ON session_logs (session_id, seq)",
    )
    .execute(pool)
    .await?;

    // ── Auth tables ──

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            github_id BIGINT NOT NULL UNIQUE,
            github_login TEXT NOT NULL,
            github_name TEXT,
            github_avatar_url TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_auth_sessions_user_id ON auth_sessions (user_id)")
        .execute(pool)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_credentials (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            name TEXT NOT NULL,
            encrypted_value TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(user_id, name)
        )",
    )
    .execute(pool)
    .await?;

    // Add user_id column to sessions if it doesn't exist.
    // SQLite doesn't support IF NOT EXISTS on ALTER TABLE, so check first.
    let has_user_id = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_table_info('sessions') WHERE name = 'user_id'",
    )
    .fetch_optional(pool)
    .await;

    // For SQLite: use pragma check; for Postgres: use information_schema
    if matches!(has_user_id, Ok(None) | Err(_)) {
        // Try the ALTER — ignore error if column already exists (Postgres)
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN user_id TEXT")
            .execute(pool)
            .await;
    }

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id)")
        .execute(pool)
        .await?;

    // Issue #14 phase 2: link sessions to the artifact they're shaping.
    // Try the ALTERs unconditionally; the errors are swallowed when the
    // columns already exist (both SQLite and Postgres return a distinct
    // error for duplicate columns, which we don't surface here).
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN artifact_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN artifact_version INTEGER")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_artifact_id ON sessions (artifact_id)")
        .execute(pool)
        .await?;

    // Issue #31: idempotency key for POST /api/shaping. Same swallow-on-
    // duplicate pattern as above for cross-backend ALTER compatibility.
    //
    // The index is UNIQUE so the database enforces at-most-one session per
    // key — the application lookup is just the fast path; concurrent inserts
    // with the same key are caught by a unique-violation at commit and
    // translated back to "return existing session". Both SQLite and Postgres
    // treat NULL values as distinct in a unique index, so sessions without an
    // idempotency key don't collide.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN idempotency_key TEXT")
        .execute(pool)
        .await;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_idempotency_key \
         ON sessions (idempotency_key)",
    )
    .execute(pool)
    .await?;

    // ── Tenant / membership / GitHub App / project tables (issue #59) ──

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tenants (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tenant_members (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            joined_at TEXT NOT NULL,
            PRIMARY KEY (tenant_id, user_id)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_tenant_members_user_id ON tenant_members (user_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS github_app_installations (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            install_id BIGINT NOT NULL UNIQUE,
            account_login TEXT NOT NULL,
            account_type TEXT NOT NULL,
            webhook_secret_cipher TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_github_app_installations_tenant_id \
         ON github_app_installations (tenant_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            github_app_installation_id TEXT NOT NULL,
            repo_owner TEXT NOT NULL,
            repo_name TEXT NOT NULL,
            default_branch TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(tenant_id, repo_owner, repo_name)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_projects_tenant_id ON projects (tenant_id)")
        .execute(pool)
        .await?;

    // Attach sessions to projects (nullable; pre-existing sessions stay personal).
    // Same swallow-on-duplicate ALTER pattern as earlier migrations for
    // cross-backend compatibility.
    let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN project_id TEXT")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions (project_id)")
        .execute(pool)
        .await?;

    Ok(())
}

// ── Node CRUD ──

pub async fn upsert_node(pool: &AnyPool, node: &Node) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO nodes (id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT(id) DO UPDATE SET
            name = $2, hostname = $3, status = $4, max_sessions = $5,
            active_sessions = $6, last_heartbeat = $7",
    )
    .bind(&node.id)
    .bind(&node.name)
    .bind(&node.hostname)
    .bind(node.status.to_string())
    .bind(node.max_sessions as i32)
    .bind(node.active_sessions as i32)
    .bind(node.last_heartbeat.to_rfc3339())
    .bind(node.registered_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_nodes(pool: &AnyPool) -> anyhow::Result<Vec<Node>> {
    let rows = sqlx::query_as::<_, NodeRow>("SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn update_node_heartbeat(
    pool: &AnyPool,
    node_id: &str,
    active_sessions: u32,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET last_heartbeat = $1, active_sessions = $2 WHERE id = $3")
        .bind(Utc::now().to_rfc3339())
        .bind(active_sessions as i32)
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_node_status(
    pool: &AnyPool,
    node_id: &str,
    status: NodeStatus,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET status = $1 WHERE id = $2")
        .bind(status.to_string())
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_least_loaded_node(pool: &AnyPool) -> anyhow::Result<Option<Node>> {
    // Exclude stale nodes (no heartbeat in last 2 minutes) to avoid dispatching
    // to dead nodes whose hostname changed across redeploys.
    let cutoff = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at
         FROM nodes
         WHERE status = 'online' AND active_sessions < max_sessions AND last_heartbeat > $1
         ORDER BY CAST(active_sessions AS REAL) / CAST(max_sessions AS REAL) ASC
         LIMIT 1",
    )
    .bind(&cutoff)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn find_node_by_name(pool: &AnyPool, name: &str) -> anyhow::Result<Option<Node>> {
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_node(pool: &AnyPool, node_id: &str) -> anyhow::Result<Option<Node>> {
    let row = sqlx::query_as::<_, NodeRow>(
        "SELECT id, name, hostname, status, max_sessions, active_sessions, last_heartbeat, registered_at FROM nodes WHERE id = $1",
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

// ── Session CRUD ──

pub async fn insert_session(pool: &AnyPool, session: &Session) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               artifact_id, artifact_version, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_sessions(pool: &AnyPool) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Look up an existing session by its idempotency key.
///
/// `request_id` from a `ShapingRequest` (or the `Idempotency-Key` header) is
/// used as the key so that a Forge retry on a dropped connection collapses
/// onto the original session instead of dispatching a second agent
/// (issue #31).
pub async fn find_session_by_idempotency_key(
    pool: &AnyPool,
    key: &str,
) -> anyhow::Result<Option<Session>> {
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE idempotency_key = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

/// Insert a session bound to an idempotency key.
///
/// Returns `Ok(true)` on a fresh insert, `Ok(false)` when a row with the same
/// key already existed and the insert was skipped (via `ON CONFLICT DO
/// NOTHING`). Callers should re-lookup on `false` to recover the winning
/// session id.
///
/// The database's unique index on `idempotency_key` is the authoritative
/// guard against concurrent POSTs with the same key — the lookup-before-
/// insert path in the handler is a fast path, not a correctness barrier.
pub async fn insert_session_with_idempotency_key(
    pool: &AnyPool,
    session: &Session,
    idempotency_key: &str,
) -> anyhow::Result<bool> {
    let affected = sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               artifact_id, artifact_version, idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (idempotency_key) DO NOTHING",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(idempotency_key)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn update_session_state(
    pool: &AnyPool,
    session_id: &str,
    state: SessionState,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE sessions SET state = $1, updated_at = $2 WHERE id = $3")
        .bind(state.to_string())
        .bind(Utc::now().to_rfc3339())
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Session Logs (append-only) ──

pub async fn append_session_log(
    pool: &AnyPool,
    session_id: &str,
    chunk: &str,
    stream: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    // Use a subquery to get the next sequence number for this session
    sqlx::query(
        "INSERT INTO session_logs (id, session_id, seq, chunk, stream, created_at)
         VALUES ($1, $2, COALESCE((SELECT MAX(seq) FROM session_logs WHERE session_id = $2), 0) + 1, $3, $4, $5)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(chunk)
    .bind(stream)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Get all log chunks for a session, ordered by sequence number.
pub async fn get_session_logs(pool: &AnyPool, session_id: &str) -> anyhow::Result<Vec<LogChunk>> {
    let rows = sqlx::query_as::<_, LogChunkRow>(
        "SELECT chunk, stream, created_at FROM session_logs WHERE session_id = $1 ORDER BY seq ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// Get log chunks added after a given sequence number (for incremental SSE).
pub async fn get_session_logs_after(
    pool: &AnyPool,
    session_id: &str,
    after_seq: i64,
) -> anyhow::Result<Vec<LogChunkWithSeq>> {
    let rows = sqlx::query_as::<_, LogChunkWithSeqRow>(
        "SELECT seq, chunk, stream, created_at FROM session_logs WHERE session_id = $1 AND seq > $2 ORDER BY seq ASC",
    )
    .bind(session_id)
    .bind(after_seq)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

#[allow(dead_code)]
pub struct LogChunk {
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

#[allow(dead_code)]
pub struct LogChunkWithSeq {
    pub seq: i64,
    pub chunk: String,
    pub stream: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct LogChunkRow {
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkRow> for LogChunk {
    fn from(row: LogChunkRow) -> Self {
        LogChunk {
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct LogChunkWithSeqRow {
    seq: i32,
    chunk: String,
    stream: String,
    created_at: String,
}

impl From<LogChunkWithSeqRow> for LogChunkWithSeq {
    fn from(row: LogChunkWithSeqRow) -> Self {
        LogChunkWithSeq {
            seq: row.seq as i64,
            chunk: row.chunk,
            stream: row.stream,
            created_at: row.created_at,
        }
    }
}

// ── User CRUD ──

pub async fn upsert_user(pool: &AnyPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT(github_id) DO UPDATE SET
            github_login = $3, github_name = $4, github_avatar_url = $5, updated_at = $7",
    )
    .bind(&user.id)
    .bind(user.github_id)
    .bind(&user.github_login)
    .bind(&user.github_name)
    .bind(&user.github_avatar_url)
    .bind(user.created_at.to_rfc3339())
    .bind(user.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_by_github_id(pool: &AnyPool, github_id: i64) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE github_id = $1",
    )
    .bind(github_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn get_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, github_id, github_login, github_name, github_avatar_url, created_at, updated_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

// ── Auth Session CRUD ──

pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub user: User,
    pub expires_at: chrono::DateTime<Utc>,
    pub created_at: chrono::DateTime<Utc>,
}

pub async fn create_auth_session(
    pool: &AnyPool,
    session_id: &str,
    user_id: &str,
    expires_at: chrono::DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO auth_sessions (id, user_id, expires_at, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(expires_at.to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_auth_session(
    pool: &AnyPool,
    session_id: &str,
) -> anyhow::Result<Option<AuthSession>> {
    let row = sqlx::query_as::<_, AuthSessionRow>(
        "SELECT a.id, a.user_id, a.expires_at, a.created_at,
                u.github_id, u.github_login, u.github_name, u.github_avatar_url,
                u.created_at as user_created_at, u.updated_at as user_updated_at
         FROM auth_sessions a JOIN users u ON a.user_id = u.id
         WHERE a.id = $1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };

    let expires_at = chrono::DateTime::parse_from_rfc3339(&row.expires_at)?.with_timezone(&Utc);
    if expires_at < Utc::now() {
        // Expired — clean up and return None
        let _ = delete_auth_session(pool, session_id).await;
        return Ok(None);
    }

    let user = User {
        id: row.user_id.clone(),
        github_id: row.github_id,
        github_login: row.github_login,
        github_name: row.github_name,
        github_avatar_url: row.github_avatar_url,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.user_created_at)?.with_timezone(&Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&row.user_updated_at)?.with_timezone(&Utc),
    };

    Ok(Some(AuthSession {
        id: row.id,
        user_id: row.user_id,
        user,
        expires_at,
        created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
    }))
}

pub async fn delete_auth_session(pool: &AnyPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM auth_sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── User Credentials CRUD ──

pub struct UserCredential {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn set_user_credential(
    pool: &AnyPool,
    user_id: &str,
    name: &str,
    encrypted_value: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO user_credentials (id, user_id, name, encrypted_value, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $5)
         ON CONFLICT(user_id, name) DO UPDATE SET encrypted_value = $4, updated_at = $5",
    )
    .bind(&id)
    .bind(user_id)
    .bind(name)
    .bind(encrypted_value)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_credentials(
    pool: &AnyPool,
    user_id: &str,
) -> anyhow::Result<Vec<UserCredential>> {
    let rows = sqlx::query_as::<_, UserCredentialRow>(
        "SELECT name, created_at, updated_at FROM user_credentials WHERE user_id = $1 ORDER BY name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UserCredential {
            name: r.name,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn get_user_credential_value(
    pool: &AnyPool,
    user_id: &str,
    name: &str,
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT encrypted_value FROM user_credentials WHERE user_id = $1 AND name = $2",
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_all_user_credential_values(
    pool: &AnyPool,
    user_id: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, CredentialKvRow>(
        "SELECT name, encrypted_value FROM user_credentials WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.name, r.encrypted_value))
        .collect())
}

pub async fn delete_user_credential(
    pool: &AnyPool,
    user_id: &str,
    name: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM user_credentials WHERE user_id = $1 AND name = $2")
        .bind(user_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Updated Session queries (user-scoped) ──

pub async fn insert_session_with_user(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
) -> anyhow::Result<()> {
    insert_session_with_user_and_project(pool, session, user_id, None).await
}

/// Variant that also binds a `project_id` when the session is scoped to a
/// tenant-owned project (issue #59). Pre-existing sessions with a null
/// `project_id` remain personal sessions forever.
pub async fn insert_session_with_user_and_project(
    pool: &AnyPool,
    session: &Session,
    user_id: Option<&str>,
    project_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id, task_id, node_id, state, prompt, output, working_dir, \
                               user_id, project_id, artifact_id, artifact_version, \
                               created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&session.id)
    .bind(&session.task_id)
    .bind(&session.node_id)
    .bind(session.state.to_string())
    .bind(&session.prompt)
    .bind(&session.output)
    .bind(&session.working_dir)
    .bind(user_id)
    .bind(project_id)
    .bind(&session.artifact_id)
    .bind(session.artifact_version)
    .bind(session.created_at.to_rfc3339())
    .bind(session.updated_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_sessions_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT id, task_id, node_id, state, prompt, output, working_dir, \
                artifact_id, artifact_version, created_at, updated_at \
         FROM sessions WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_session_owner(pool: &AnyPool, session_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM sessions WHERE id = $1 AND user_id IS NOT NULL",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ── Row types for sqlx ──

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: String,
    name: String,
    hostname: String,
    status: String,
    max_sessions: i32,
    active_sessions: i32,
    last_heartbeat: String,
    registered_at: String,
}

impl TryFrom<NodeRow> for Node {
    type Error = anyhow::Error;

    fn try_from(row: NodeRow) -> anyhow::Result<Self> {
        Ok(Node {
            id: row.id,
            name: row.name,
            hostname: row.hostname,
            status: row
                .status
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            max_sessions: row.max_sessions as u32,
            active_sessions: row.active_sessions as u32,
            last_heartbeat: chrono::DateTime::parse_from_rfc3339(&row.last_heartbeat)?
                .with_timezone(&Utc),
            registered_at: chrono::DateTime::parse_from_rfc3339(&row.registered_at)?
                .with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    task_id: String,
    node_id: String,
    state: String,
    prompt: String,
    output: Option<String>,
    working_dir: Option<String>,
    artifact_id: Option<String>,
    artifact_version: Option<i32>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<SessionRow> for Session {
    type Error = anyhow::Error;

    fn try_from(row: SessionRow) -> anyhow::Result<Self> {
        Ok(Session {
            id: row.id,
            task_id: row.task_id,
            node_id: row.node_id,
            state: row
                .state
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            prompt: row.prompt,
            output: row.output,
            working_dir: row.working_dir,
            artifact_id: row.artifact_id,
            artifact_version: row.artifact_version,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<UserRow> for User {
    type Error = anyhow::Error;

    fn try_from(row: UserRow) -> anyhow::Result<Self> {
        Ok(User {
            id: row.id,
            github_id: row.github_id,
            github_login: row.github_login,
            github_name: row.github_name,
            github_avatar_url: row.github_avatar_url,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.updated_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuthSessionRow {
    id: String,
    user_id: String,
    expires_at: String,
    created_at: String,
    // User fields from join
    github_id: i64,
    github_login: String,
    github_name: Option<String>,
    github_avatar_url: Option<String>,
    user_created_at: String,
    user_updated_at: String,
}

#[derive(sqlx::FromRow)]
struct UserCredentialRow {
    name: String,
    created_at: String,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct CredentialKvRow {
    name: String,
    encrypted_value: String,
}

// ── Tenant / membership / installation / project CRUD (issue #59) ──

pub async fn insert_tenant(pool: &AnyPool, tenant: &Tenant) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tenants (id, slug, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&tenant.id)
    .bind(&tenant.slug)
    .bind(&tenant.name)
    .bind(&tenant.created_by)
    .bind(tenant.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_tenant(pool: &AnyPool, tenant_id: &str) -> anyhow::Result<Option<Tenant>> {
    let row = sqlx::query_as::<_, TenantRow>(
        "SELECT id, slug, name, created_by, created_at FROM tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_tenants_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Tenant>> {
    let rows = sqlx::query_as::<_, TenantRow>(
        "SELECT t.id, t.slug, t.name, t.created_by, t.created_at \
         FROM tenants t \
         JOIN tenant_members m ON t.id = m.tenant_id \
         WHERE m.user_id = $1 \
         ORDER BY t.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn insert_tenant_member(pool: &AnyPool, member: &TenantMember) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tenant_members (tenant_id, user_id, joined_at) VALUES ($1, $2, $3)")
        .bind(&member.tenant_id)
        .bind(&member.user_id)
        .bind(member.joined_at.to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn is_tenant_member(
    pool: &AnyPool,
    tenant_id: &str,
    user_id: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM tenant_members WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub async fn list_tenant_members(
    pool: &AnyPool,
    tenant_id: &str,
) -> anyhow::Result<Vec<TenantMember>> {
    let rows = sqlx::query_as::<_, TenantMemberRow>(
        "SELECT tenant_id, user_id, joined_at \
         FROM tenant_members WHERE tenant_id = $1 ORDER BY joined_at ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn insert_github_app_installation(
    pool: &AnyPool,
    install: &GitHubAppInstallation,
    webhook_secret_cipher: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO github_app_installations (id, tenant_id, install_id, account_login, \
                                               account_type, webhook_secret_cipher, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&install.id)
    .bind(&install.tenant_id)
    .bind(install.install_id)
    .bind(&install.account_login)
    .bind(install.account_type.to_string())
    .bind(webhook_secret_cipher)
    .bind(install.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_github_app_installations_for_tenant(
    pool: &AnyPool,
    tenant_id: &str,
) -> anyhow::Result<Vec<GitHubAppInstallation>> {
    let rows = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, tenant_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE tenant_id = $1 ORDER BY created_at ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn get_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<Option<GitHubAppInstallation>> {
    let row = sqlx::query_as::<_, GitHubAppInstallationRow>(
        "SELECT id, tenant_id, install_id, account_login, account_type, created_at \
         FROM github_app_installations WHERE id = $1",
    )
    .bind(install_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn delete_github_app_installation(
    pool: &AnyPool,
    install_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM github_app_installations WHERE id = $1")
        .bind(install_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_project(pool: &AnyPool, project: &Project) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO projects (id, tenant_id, github_app_installation_id, repo_owner, \
                               repo_name, default_branch, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&project.id)
    .bind(&project.tenant_id)
    .bind(&project.github_app_installation_id)
    .bind(&project.repo_owner)
    .bind(&project.repo_name)
    .bind(&project.default_branch)
    .bind(project.created_at.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<Option<Project>> {
    let row = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, tenant_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.try_into()).transpose()
}

pub async fn list_projects_for_tenant(
    pool: &AnyPool,
    tenant_id: &str,
) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, tenant_id, github_app_installation_id, repo_owner, repo_name, \
                default_branch, created_at \
         FROM projects WHERE tenant_id = $1 ORDER BY created_at ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn list_projects_for_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Vec<Project>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT p.id, p.tenant_id, p.github_app_installation_id, p.repo_owner, p.repo_name, \
                p.default_branch, p.created_at \
         FROM projects p \
         JOIN tenant_members m ON p.tenant_id = m.tenant_id \
         WHERE m.user_id = $1 \
         ORDER BY p.created_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|r| r.try_into()).collect()
}

pub async fn delete_project(pool: &AnyPool, project_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count sessions attached to a project that are not in a terminal state.
/// Used to block project deletion while live sessions reference it (no
/// cascade, no soft-delete in v1).
pub async fn count_live_sessions_for_project(
    pool: &AnyPool,
    project_id: &str,
) -> anyhow::Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions \
         WHERE project_id = $1 AND state NOT IN ('done', 'failed')",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ── Row types (Tenant / membership / installation / project) ──

#[derive(sqlx::FromRow)]
struct TenantRow {
    id: String,
    slug: String,
    name: String,
    created_by: String,
    created_at: String,
}

impl TryFrom<TenantRow> for Tenant {
    type Error = anyhow::Error;

    fn try_from(row: TenantRow) -> anyhow::Result<Self> {
        Ok(Tenant {
            id: row.id,
            slug: row.slug,
            name: row.name,
            created_by: row.created_by,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct TenantMemberRow {
    tenant_id: String,
    user_id: String,
    joined_at: String,
}

impl TryFrom<TenantMemberRow> for TenantMember {
    type Error = anyhow::Error;

    fn try_from(row: TenantMemberRow) -> anyhow::Result<Self> {
        Ok(TenantMember {
            tenant_id: row.tenant_id,
            user_id: row.user_id,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.joined_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct GitHubAppInstallationRow {
    id: String,
    tenant_id: String,
    install_id: i64,
    account_login: String,
    account_type: String,
    created_at: String,
}

impl TryFrom<GitHubAppInstallationRow> for GitHubAppInstallation {
    type Error = anyhow::Error;

    fn try_from(row: GitHubAppInstallationRow) -> anyhow::Result<Self> {
        Ok(GitHubAppInstallation {
            id: row.id,
            tenant_id: row.tenant_id,
            install_id: row.install_id,
            account_login: row.account_login,
            account_type: row
                .account_type
                .parse()
                .map_err(|e: crate::core::StiglabError| anyhow::anyhow!(e))?,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    tenant_id: String,
    github_app_installation_id: String,
    repo_owner: String,
    repo_name: String,
    default_branch: String,
    created_at: String,
}

impl TryFrom<ProjectRow> for Project {
    type Error = anyhow::Error;

    fn try_from(row: ProjectRow) -> anyhow::Result<Self> {
        Ok(Project {
            id: row.id,
            tenant_id: row.tenant_id,
            github_app_installation_id: row.github_app_installation_id,
            repo_owner: row.repo_owner,
            repo_name: row.repo_name,
            default_branch: row.default_branch,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{GitHubAccountType, Project, Session, SessionState, Tenant, TenantMember};
    use chrono::Utc;
    use uuid::Uuid;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = PoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite in-memory");
        run_migrations(&pool)
            .await
            .expect("migrations should succeed");
        pool
    }

    async fn seed_user(pool: &AnyPool, user_id: &str) {
        // Derive a stable non-colliding github_id from the user_id bytes.
        let github_id: i64 = user_id.bytes().fold(0i64, |acc, b| acc * 131 + b as i64);
        sqlx::query(
            "INSERT INTO users (id, github_id, github_login, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(user_id)
        .bind(github_id)
        .bind(user_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    fn new_tenant(created_by: &str) -> Tenant {
        Tenant {
            id: Uuid::new_v4().to_string(),
            slug: format!("tenant-{}", Uuid::new_v4().simple()),
            name: "Test Tenant".to_string(),
            created_by: created_by.to_string(),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn tenant_crud_roundtrip() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;

        let tenant = new_tenant("u1");
        insert_tenant(&pool, &tenant).await.unwrap();

        let fetched = get_tenant(&pool, &tenant.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, tenant.id);
        assert_eq!(fetched.slug, tenant.slug);
    }

    #[tokio::test]
    async fn membership_query_and_list_tenants_for_user() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let t = new_tenant("u1");
        insert_tenant(&pool, &t).await.unwrap();
        insert_tenant_member(
            &pool,
            &TenantMember {
                tenant_id: t.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        assert!(is_tenant_member(&pool, &t.id, "u1").await.unwrap());
        assert!(!is_tenant_member(&pool, &t.id, "u2").await.unwrap());

        let u1_tenants = list_tenants_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_tenants.len(), 1);
        assert_eq!(u1_tenants[0].id, t.id);

        let u2_tenants = list_tenants_for_user(&pool, "u2").await.unwrap();
        assert!(u2_tenants.is_empty());
    }

    #[tokio::test]
    async fn installation_and_project_crud() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let t = new_tenant("u1");
        insert_tenant(&pool, &t).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            tenant_id: t.id.clone(),
            install_id: 42,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, Some("ciphertext"))
            .await
            .unwrap();

        let installs = list_github_app_installations_for_tenant(&pool, &t.id)
            .await
            .unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].install_id, 42);

        let project = Project {
            id: Uuid::new_v4().to_string(),
            tenant_id: t.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let projects = list_projects_for_tenant(&pool, &t.id).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].repo_name, "widgets");
    }

    #[tokio::test]
    async fn delete_project_blocks_on_live_sessions() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let t = new_tenant("u1");
        insert_tenant(&pool, &t).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            tenant_id: t.id.clone(),
            install_id: 7,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();

        let project = Project {
            id: Uuid::new_v4().to_string(),
            tenant_id: t.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let session = Session {
            id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            node_id: "node-1".to_string(),
            state: SessionState::Running,
            prompt: "hello".to_string(),
            output: None,
            working_dir: None,
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        insert_session_with_user_and_project(&pool, &session, Some("u1"), Some(&project.id))
            .await
            .unwrap();

        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 1);

        // Transition to a terminal state — live count should drop to zero.
        update_session_state(&pool, &session.id, SessionState::Done)
            .await
            .unwrap();
        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 0);

        delete_project(&pool, &project.id).await.unwrap();
        assert!(get_project(&pool, &project.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_projects_for_user_follows_membership() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let t1 = new_tenant("u1");
        insert_tenant(&pool, &t1).await.unwrap();
        insert_tenant_member(
            &pool,
            &TenantMember {
                tenant_id: t1.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            tenant_id: t1.id.clone(),
            install_id: 1,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::User,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            tenant_id: t1.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let u1_projects = list_projects_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_projects.len(), 1);

        let u2_projects = list_projects_for_user(&pool, "u2").await.unwrap();
        assert!(u2_projects.is_empty());
    }
}
