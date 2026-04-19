use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgListener, PgPool, PgPoolOptions};
use tokio::sync::mpsc;

use crate::extension_event::ExtensionEventRecord;
use crate::factory_event::FactoryEvent;

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

    /// Append a factory event to the event spine.
    /// Returns the assigned event id.
    pub async fn append_factory_event(
        &self,
        event: &FactoryEvent,
        metadata: &EventMetadata,
    ) -> Result<i64, sqlx::Error> {
        let stream_id = event.event.stream_id();
        let stream_type = event.event.stream_type();
        let event_type = event.event.event_type();
        let data = serde_json::to_value(event).expect("FactoryEvent must be serializable");
        let meta = serde_json::to_value(metadata).expect("EventMetadata must be serializable");

        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO events (stream_id, stream_type, event_type, data, metadata, sequence)
            VALUES ($1, $2, $3, $4, $5,
                    COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE stream_id = $1), 1))
            RETURNING id
            "#,
        )
        .bind(&stream_id)
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
        let meta = serde_json::to_value(metadata).expect("EventMetadata must be serializable");

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

    /// Fetch a single core event by id. Returns `None` if not found.
    pub async fn get_event_by_id(&self, id: i64) -> Result<Option<EventRecord>, sqlx::Error> {
        sqlx::query_as::<_, EventRecord>(
            r#"
            SELECT id, stream_id, stream_type, event_type, data, metadata, sequence, created_at
            FROM events
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    /// Return the highest id present in either `events` or `events_ext`,
    /// or `None` if both tables are empty. Callers use this as a
    /// warm-start cursor so a [`Listener`] with `with_since` skips
    /// backfill over history it doesn't need to replay.
    pub async fn max_event_id(&self) -> Result<Option<i64>, sqlx::Error> {
        let row: (Option<i64>,) = sqlx::query_as(
            "SELECT GREATEST( \
                (SELECT MAX(id) FROM events), \
                (SELECT MAX(id) FROM events_ext) \
             )",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Fetch a single extension event by id. Returns `None` if not found.
    pub async fn get_ext_event_by_id(
        &self,
        id: i64,
    ) -> Result<Option<ExtensionEventRecord>, sqlx::Error> {
        sqlx::query_as::<_, ExtensionEventRecord>(
            r#"
            SELECT id, stream_id, namespace, event_type, data, metadata, ref_event_id, created_at
            FROM events_ext
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
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
    /// Returns a receiver that yields [`EventNotification`] as events are inserted.
    ///
    /// **Warning**: this uses an unbounded channel. A slow consumer can cause
    /// unbounded memory growth. Prefer [`EventStore::subscribe_bounded`] when
    /// backpressure is required.
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

    /// Subscribe to real-time event notifications with a bounded channel.
    ///
    /// Unlike [`EventStore::subscribe`], this applies backpressure: the sender
    /// task blocks when the channel is full. Use this when the consumer may be
    /// slower than the event rate and unbounded memory growth is unacceptable.
    ///
    /// `capacity` is the maximum number of buffered notifications. Choose a
    /// value large enough to absorb bursts without causing the sender to block
    /// excessively.
    pub async fn subscribe_bounded(
        &self,
        capacity: usize,
    ) -> Result<mpsc::Receiver<EventNotification>, sqlx::Error> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen("onsager_events").await?;

        let (tx, rx) = mpsc::channel(capacity);

        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(notification) => {
                        if let Ok(parsed) =
                            serde_json::from_str::<EventNotification>(notification.payload())
                        {
                            if tx.send(parsed).await.is_err() {
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

    /// Run a closure inside a single PostgreSQL transaction.
    ///
    /// The transaction is committed if the closure returns `Ok`, and rolled back
    /// if it returns `Err` or if the future is dropped before completion.
    /// Use [`append_factory_event_tx`] inside the closure to append events within
    /// the same transaction as state mutations.
    pub async fn transaction<F, R>(&self, f: F) -> Result<R, sqlx::Error>
    where
        F: for<'c> FnOnce(
            &'c mut sqlx::Transaction<'_, sqlx::Postgres>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<R, sqlx::Error>> + Send + 'c>,
        >,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Get the underlying pool (for advanced queries).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// Append a factory event inside an existing transaction.
///
/// Use this inside [`EventStore::transaction`] to append events atomically with
/// state mutations.
pub async fn append_factory_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &FactoryEvent,
    metadata: &EventMetadata,
) -> Result<i64, sqlx::Error> {
    let stream_id = event.event.stream_id();
    let stream_type = event.event.stream_type();
    let event_type = event.event.event_type();
    let data = serde_json::to_value(event).expect("FactoryEvent must be serializable");
    let meta = serde_json::to_value(metadata).expect("EventMetadata must be serializable");

    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO events (stream_id, stream_type, event_type, data, metadata, sequence)
        VALUES ($1, $2, $3, $4, $5,
                COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE stream_id = $1), 1))
        RETURNING id
        "#,
    )
    .bind(&stream_id)
    .bind(stream_type)
    .bind(event_type)
    .bind(&data)
    .bind(&meta)
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.0)
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::factory_event::{FactoryEvent, FactoryEventKind};
    use onsager_artifact::{ArtifactId, Kind};

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    fn test_event(stream_suffix: &str) -> FactoryEvent {
        FactoryEvent {
            event: FactoryEventKind::ArtifactRegistered {
                artifact_id: ArtifactId::new(format!("art_tx_test_{stream_suffix}")),
                kind: Kind::Document,
                name: "tx test".into(),
                owner: "test".into(),
            },
            correlation_id: None,
            causation_id: None,
            actor: "test".into(),
            timestamp: Utc::now(),
        }
    }

    fn test_metadata() -> EventMetadata {
        EventMetadata {
            actor: "test".into(),
            ..Default::default()
        }
    }

    /// Committed transaction: event lands in the database.
    #[tokio::test]
    async fn transaction_commit_persists_event() {
        let Some(url) = db_url() else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };
        let store = EventStore::connect(&url).await.unwrap();
        let event = test_event("commit");
        let stream_id = event.event.stream_id();
        let meta = test_metadata();

        let id = store
            .transaction(|tx| {
                Box::pin(async move { append_factory_event_tx(tx, &event, &meta).await })
            })
            .await
            .unwrap();

        // Verify the row is present after commit.
        let rows = store.query_stream(&stream_id, 1).await.unwrap();
        assert!(
            rows.iter().any(|r| r.id == id),
            "event {id} not found after commit"
        );

        // Clean up.
        sqlx::query("DELETE FROM events WHERE id = $1")
            .bind(id)
            .execute(store.pool())
            .await
            .unwrap();
    }

    /// Rolled-back transaction: event is not visible after rollback.
    #[tokio::test]
    async fn transaction_rollback_discards_event() {
        let Some(url) = db_url() else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };
        let store = EventStore::connect(&url).await.unwrap();
        let event = test_event("rollback");
        let stream_id = event.event.stream_id();
        let meta = test_metadata();

        let result: Result<i64, sqlx::Error> = store
            .transaction(|tx| {
                Box::pin(async move {
                    let _id = append_factory_event_tx(tx, &event, &meta).await?;
                    Err(sqlx::Error::RowNotFound) // trigger rollback
                })
            })
            .await;

        assert!(result.is_err());
        // Verify no rows landed.
        let rows = store.query_stream(&stream_id, 1).await.unwrap();
        assert!(
            rows.is_empty(),
            "found unexpected rows after rollback: {rows:?}"
        );
    }

    /// Dropped transaction (simulating panic path): event does not persist.
    #[tokio::test]
    async fn transaction_drop_rolls_back() {
        let Some(url) = db_url() else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };
        let store = EventStore::connect(&url).await.unwrap();
        let event = test_event("drop");
        let stream_id = event.event.stream_id();
        let meta = test_metadata();

        // Manually begin a transaction, append, then drop without committing.
        {
            let mut tx = store.pool().begin().await.unwrap();
            let _ = append_factory_event_tx(&mut tx, &event, &meta)
                .await
                .unwrap();
            // tx dropped here — no commit
        }

        // Verify no rows landed.
        let rows = store.query_stream(&stream_id, 1).await.unwrap();
        assert!(
            rows.is_empty(),
            "found unexpected rows after drop (no commit): {rows:?}"
        );
    }
}
