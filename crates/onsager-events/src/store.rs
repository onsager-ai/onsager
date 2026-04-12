use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgListener, PgPool, PgPoolOptions};
use tokio::sync::mpsc;

use crate::core_event::CoreEvent;
use crate::extension_event::ExtensionEventRecord;

/// An event record as stored in the `events` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventRecord {
    pub id: i64,
    pub stream_id: String,
    pub stream_type: String,
    pub event_type: String,
    pub data: serde_json::Value,
    pub metadata: serde_json::Value,
    pub sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// Metadata attached to every event for traceability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<i64>,
    pub actor: String,
}

/// Notification received from pg_notify when new events are inserted.
#[derive(Debug, Clone, Deserialize)]
pub struct EventNotification {
    pub table: String,
    pub id: i64,
    pub stream_id: String,
    pub event_type: String,
}

/// The PostgreSQL-backed event store — the spine of Onsager.
#[derive(Clone)]
pub struct EventStore {
    pool: PgPool,
}

impl EventStore {
    /// Connect to PostgreSQL and return an EventStore.
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Run database migrations (creates tables, indexes, triggers).
    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        let migration_sql = include_str!("../../../migrations/001_initial.sql");
        sqlx::raw_sql(migration_sql).execute(&self.pool).await?;
        Ok(())
    }

    /// Append a core event to the event stream.
    /// Returns the assigned event id.
    pub async fn append(
        &self,
        event: &CoreEvent,
        metadata: &EventMetadata,
    ) -> Result<i64, sqlx::Error> {
        let stream_id = event.stream_id();
        let stream_type = event.stream_type();
        let event_type = event.event_type();
        let data = serde_json::to_value(event).unwrap_or_default();
        let meta = serde_json::to_value(metadata).unwrap_or_default();

        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO events (stream_id, stream_type, event_type, data, metadata, sequence)
            VALUES ($1, $2, $3, $4, $5,
                    COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE stream_id = $1), 1))
            RETURNING id
            "#,
        )
        .bind(stream_id)
        .bind(stream_type)
        .bind(event_type)
        .bind(&data)
        .bind(&meta)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Append an extension event linked to a stream.
    /// Returns the assigned event id.
    pub async fn append_ext(
        &self,
        stream_id: &str,
        namespace: &str,
        event_type: &str,
        data: serde_json::Value,
        metadata: &EventMetadata,
        ref_event_id: Option<i64>,
    ) -> Result<i64, sqlx::Error> {
        let meta = serde_json::to_value(metadata).unwrap_or_default();

        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO events_ext (stream_id, namespace, event_type, data, metadata, ref_event_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(stream_id)
        .bind(namespace)
        .bind(event_type)
        .bind(&data)
        .bind(&meta)
        .bind(ref_event_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Query core events for a stream, ordered by sequence.
    pub async fn query_stream(
        &self,
        stream_id: &str,
        from_sequence: i64,
    ) -> Result<Vec<EventRecord>, sqlx::Error> {
        sqlx::query_as::<_, EventRecord>(
            r#"
            SELECT id, stream_id, stream_type, event_type, data, metadata, sequence, created_at
            FROM events
            WHERE stream_id = $1 AND sequence >= $2
            ORDER BY sequence ASC
            "#,
        )
        .bind(stream_id)
        .bind(from_sequence)
        .fetch_all(&self.pool)
        .await
    }

    /// Query extension events for a stream, ordered by creation time.
    pub async fn query_ext_stream(
        &self,
        stream_id: &str,
    ) -> Result<Vec<ExtensionEventRecord>, sqlx::Error> {
        sqlx::query_as::<_, ExtensionEventRecord>(
            r#"
            SELECT id, stream_id, namespace, event_type, data, metadata, ref_event_id, created_at
            FROM events_ext
            WHERE stream_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(stream_id)
        .fetch_all(&self.pool)
        .await
    }

    /// Query recent core events, optionally filtered.
    pub async fn query_events(
        &self,
        stream_id: Option<&str>,
        event_type: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<EventRecord>, sqlx::Error> {
        // Build a dynamic query with optional filters
        let mut conditions = vec!["TRUE".to_string()];
        if stream_id.is_some() {
            conditions.push(format!("stream_id = ${}", conditions.len() + 1));
        }
        if event_type.is_some() {
            conditions.push(format!("event_type = ${}", conditions.len() + 1));
        }
        if since.is_some() {
            conditions.push(format!("created_at >= ${}", conditions.len() + 1));
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT id, stream_id, stream_type, event_type, data, metadata, sequence, created_at \
             FROM events WHERE {where_clause} ORDER BY id DESC LIMIT {limit}"
        );

        let mut query = sqlx::query_as::<_, EventRecord>(&sql);
        if let Some(sid) = stream_id {
            query = query.bind(sid);
        }
        if let Some(et) = event_type {
            query = query.bind(et);
        }
        if let Some(s) = since {
            query = query.bind(s);
        }

        query.fetch_all(&self.pool).await
    }

    /// Query extension events, optionally filtered by namespace.
    pub async fn query_ext_events(
        &self,
        stream_id: Option<&str>,
        namespace: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ExtensionEventRecord>, sqlx::Error> {
        let mut conditions = vec!["TRUE".to_string()];
        if stream_id.is_some() {
            conditions.push(format!("stream_id = ${}", conditions.len() + 1));
        }
        if namespace.is_some() {
            conditions.push(format!("namespace = ${}", conditions.len() + 1));
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT id, stream_id, namespace, event_type, data, metadata, ref_event_id, created_at \
             FROM events_ext WHERE {where_clause} ORDER BY id DESC LIMIT {limit}"
        );

        let mut query = sqlx::query_as::<_, ExtensionEventRecord>(&sql);
        if let Some(sid) = stream_id {
            query = query.bind(sid);
        }
        if let Some(ns) = namespace {
            query = query.bind(ns);
        }

        query.fetch_all(&self.pool).await
    }

    /// Subscribe to real-time event notifications via pg_notify.
    /// Returns a receiver that yields EventNotification as events are inserted.
    pub async fn subscribe(
        &self,
    ) -> Result<mpsc::UnboundedReceiver<EventNotification>, sqlx::Error> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen("onsager_events").await?;

        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(notification) => {
                        if let Ok(parsed) =
                            serde_json::from_str::<EventNotification>(notification.payload())
                        {
                            if tx.send(parsed).is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("pg_notify listener error: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    /// Get the underlying pool (for advanced queries).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

// Implement FromRow for ExtensionEventRecord manually since it has Option<i64>
impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for ExtensionEventRecord {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(ExtensionEventRecord {
            id: row.try_get("id")?,
            stream_id: row.try_get("stream_id")?,
            namespace: row.try_get("namespace")?,
            event_type: row.try_get("event_type")?,
            data: row.try_get("data")?,
            metadata: row.try_get("metadata")?,
            ref_event_id: row.try_get("ref_event_id")?,
            created_at: row.try_get("created_at")?,
        })
    }
}
