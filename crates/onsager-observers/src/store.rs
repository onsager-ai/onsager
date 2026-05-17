//! Postgres-backed persistence for [`ObserverOutput`] rows.
//!
//! Schema lives at
//! `crates/onsager-spine/migrations/028_observer_outputs.sql`. One
//! row per emitted [`ObserverOutput`] — the runtime calls
//! [`ObserverOutputStore::record`] from inside the per-event task it
//! spawns, so persistence stays off the substrate scheduler's hot
//! path.
//!
//! The store is deliberately separate from the spine `EventStore`:
//! observers do not write to `events` / `events_ext` (ADR 0013
//! "cannot modify state"). Persistence here is one-way — observers
//! emit, the dashboard reads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, postgres::PgRow};
use thiserror::Error;

use crate::output::{ObserverOutput, ObserverOutputKind};

/// A persisted [`ObserverOutput`] row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverOutputRecord {
    pub id: i64,
    pub observer_id: String,
    pub kind: ObserverOutputKind,
    /// `events.id` of the row that triggered this output; `None` for
    /// outputs not anchored to a specific event (rare — most
    /// observers should carry the triggering event id).
    pub triggered_by_event_id: Option<i64>,
    /// JSON-encoded [`ObserverOutput`] payload.
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl ObserverOutputRecord {
    /// Parse the JSON payload back into a typed [`ObserverOutput`].
    pub fn into_output(self) -> Result<ObserverOutput, serde_json::Error> {
        serde_json::from_value(self.payload)
    }
}

impl<'r> sqlx::FromRow<'r, PgRow> for ObserverOutputRecord {
    fn from_row(row: &'r PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        let kind_str: String = row.try_get("kind")?;
        let kind = match kind_str.as_str() {
            "quality_signal" => ObserverOutputKind::QualitySignal,
            "insight" => ObserverOutputKind::Insight,
            "alert" => ObserverOutputKind::Alert,
            other => {
                return Err(sqlx::Error::ColumnDecode {
                    index: "kind".into(),
                    source: Box::new(StoreError::UnknownKind(other.to_string())),
                });
            }
        };
        Ok(Self {
            id: row.try_get("id")?,
            observer_id: row.try_get("observer_id")?,
            kind,
            triggered_by_event_id: row.try_get("triggered_by_event_id")?,
            payload: row.try_get("payload")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

/// Errors returned by the observer-output persistence layer.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("serializing ObserverOutput failed: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("unknown observer-output kind `{0}` in `observer_outputs.kind` column")]
    UnknownKind(String),
}

/// Postgres-backed observer-output store. Cheap to clone.
#[derive(Clone)]
pub struct ObserverOutputStore {
    pool: PgPool,
}

impl ObserverOutputStore {
    /// Wrap an existing pool. The pool must point at a database with
    /// the spine migrations applied (see
    /// `crates/onsager-spine/migrations/028_observer_outputs.sql`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool for advanced reads.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Persist one [`ObserverOutput`] under `observer_id`, optionally
    /// linking back to the spine event id that triggered it. Returns
    /// the assigned row id.
    pub async fn record(
        &self,
        observer_id: &str,
        triggered_by_event_id: Option<i64>,
        output: &ObserverOutput,
    ) -> Result<i64, StoreError> {
        let payload = serde_json::to_value(output)?;
        let kind = output.kind().as_str();
        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO observer_outputs (observer_id, kind, triggered_by_event_id, payload)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(observer_id)
        .bind(kind)
        .bind(triggered_by_event_id)
        .bind(payload)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// List rows for one observer, newest first, capped at `limit`.
    pub async fn list_by_observer(
        &self,
        observer_id: &str,
        limit: i64,
    ) -> Result<Vec<ObserverOutputRecord>, StoreError> {
        let rows = sqlx::query_as::<_, ObserverOutputRecord>(
            r#"
            SELECT id, observer_id, kind, triggered_by_event_id, payload, created_at
            FROM observer_outputs
            WHERE observer_id = $1
            ORDER BY id DESC
            LIMIT $2
            "#,
        )
        .bind(observer_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List rows for one observer-output kind, newest first, capped
    /// at `limit`. Useful for the dashboard's "all alerts" view.
    pub async fn list_by_kind(
        &self,
        kind: ObserverOutputKind,
        limit: i64,
    ) -> Result<Vec<ObserverOutputRecord>, StoreError> {
        let rows = sqlx::query_as::<_, ObserverOutputRecord>(
            r#"
            SELECT id, observer_id, kind, triggered_by_event_id, payload, created_at
            FROM observer_outputs
            WHERE kind = $1
            ORDER BY id DESC
            LIMIT $2
            "#,
        )
        .bind(kind.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{Alert, AlertSeverity, Insight};
    use onsager_artifact::{QualitySignal, QualitySource, QualityValue};

    fn make_quality_signal() -> QualitySignal {
        QualitySignal {
            source: QualitySource::IsingInference,
            dimension: "completeness".into(),
            value: QualityValue::Score(0.42),
            recorded_at: Utc::now(),
            recorded_by: "obs:demo".into(),
        }
    }

    /// Roundtrip: ObserverOutputRecord JSON payload <-> ObserverOutput.
    #[test]
    fn record_payload_roundtrip_for_each_variant() {
        let cases = [
            ObserverOutput::Insight(Insight::new("flaky test", 0.7)),
            ObserverOutput::Alert(Alert::new("SLA crossed", "boom", AlertSeverity::Warning)),
            ObserverOutput::QualitySignal(make_quality_signal()),
        ];
        for ev in cases {
            let payload = serde_json::to_value(&ev).unwrap();
            let back: ObserverOutput = serde_json::from_value(payload).unwrap();
            assert_eq!(back.kind(), ev.kind());
        }
    }

    /// Integration test against a real Postgres. Skipped unless
    /// `DATABASE_URL` is set — matches the spine listener test.
    #[tokio::test]
    async fn record_and_read_back() {
        let Some(db_url) = std::env::var("DATABASE_URL").ok() else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };
        let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
        let store = ObserverOutputStore::new(pool.clone());

        let observer_id = format!("test_observer_{}", ulid::Ulid::new());
        let insight = ObserverOutput::Insight(Insight::new("seed observation", 0.9));
        // `triggered_by_event_id = None` keeps this unit test
        // independent of the spine `events` table — the FK on the
        // column allows NULL exactly for this case.
        let id = store.record(&observer_id, None, &insight).await.unwrap();
        assert!(id > 0);

        let rows = store.list_by_observer(&observer_id, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, ObserverOutputKind::Insight);
        assert_eq!(rows[0].triggered_by_event_id, None);
        let parsed = rows[0].clone().into_output().unwrap();
        assert_eq!(parsed, insight);

        // Cleanup.
        sqlx::query("DELETE FROM observer_outputs WHERE observer_id = $1")
            .bind(&observer_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
