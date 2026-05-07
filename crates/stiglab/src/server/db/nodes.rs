use crate::core::{Node, NodeStatus};
use chrono::Utc;
use sqlx::AnyPool;

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

// ── Row types ──

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
