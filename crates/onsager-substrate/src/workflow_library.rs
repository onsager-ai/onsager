//! `WorkflowLibrary` — the flat catalog mapping spec kind → `Workflow`.
//!
//! Per [ADR 0016](../../../docs/adr/0016-workflow-library-n-isomorphic-islands.md)
//! the Workflow Library is a flat catalog: one active `Workflow` per
//! spec `kind`. The Plan Compiler (SUB-05, #352) resolves
//! `kind → Workflow` via library lookup with no priority lists, no
//! defaults, and no fallback chain. Same-kind specs share the same
//! shape — the "N isomorphic islands" property.
//!
//! This issue (SUB-04, #351) adds the persistence layer the compiler
//! reads from: a Postgres-backed registry keyed by `(spec_kind,
//! version)`. Versions are monotonic per kind; the latest version is
//! the active one (ADR 0016 "one active workflow per kind"). The
//! kernel's data model — [`Workflow`], [`Node`], [`Edge`] — stays
//! pure data and lives in [`crate::workflow`]; only this module
//! reaches into the database.
//!
//! # Surface
//!
//! - [`WorkflowLibrary::register`] inserts a new version for a kind
//!   and returns the assigned version number. Versions are monotonic
//!   per kind (next = `MAX(version) + 1`).
//! - [`WorkflowLibrary::lookup`] and [`WorkflowLibrary::latest`] both
//!   return the latest registered workflow for a kind — they're the
//!   compiler-facing read path. They return `Ok(None)` when the kind
//!   has no entries.
//!
//! # Errors
//!
//! - [`WorkflowLibraryError::DuplicateKind`] surfaces the unique
//!   constraint on `(spec_kind, version)` — two concurrent
//!   registrations both computing the same next-version, or any
//!   future caller that explicitly targets a version that already
//!   exists. The compiler treats this as a non-fatal contention
//!   signal: retry the register, or re-read the latest.

use crate::workflow::Workflow;
use sqlx::PgPool;
use thiserror::Error;

/// Name of the `(spec_kind, version)` unique constraint on
/// `workflow_library`. Declared in
/// `crates/onsager-spine/migrations/027_workflow_library.sql`; matched
/// against `sqlx::error::DatabaseError::constraint()` so unrelated
/// uniqueness violations (e.g. the primary key on `id`) don't get
/// misclassified as [`WorkflowLibraryError::DuplicateKind`].
const KIND_VERSION_UNIQUE_CONSTRAINT: &str = "workflow_library_kind_version_unique";

/// Errors returned by [`WorkflowLibrary`] operations.
#[derive(Debug, Error)]
pub enum WorkflowLibraryError {
    /// A row already exists at the requested `(kind, version)` —
    /// surfaced by the table's unique constraint. See ADR 0016's
    /// "one active workflow per kind" rule.
    #[error("duplicate registration for spec kind '{kind}' at version {version}")]
    DuplicateKind { kind: String, version: i32 },

    /// The stored `workflow_json` failed to deserialize back into a
    /// `Workflow`. A row written by one substrate version and read
    /// by another with an incompatible schema would land here.
    #[error("workflow_json (de)serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    /// Any other database failure — connection, timeout, schema
    /// mismatch — bubbles up unchanged. The [`DuplicateKind`](Self::DuplicateKind)
    /// constraint violation is intercepted earlier; everything else
    /// reaches the caller here.
    #[error("workflow_library database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Postgres-backed Workflow Library — the SUB-04 (#351) persistence
/// layer described in ADR 0016.
///
/// Cheap to clone (wraps a `sqlx::PgPool`, which is itself
/// reference-counted). The Plan Compiler holds one of these per
/// process.
#[derive(Clone)]
pub struct WorkflowLibrary {
    pool: PgPool,
}

impl WorkflowLibrary {
    /// Wrap a pool. The pool must point at a database with the spine
    /// migrations applied (see
    /// `crates/onsager-spine/migrations/027_workflow_library.sql`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Register a new version of a workflow under `kind` and return
    /// the assigned version. Versions are monotonic per kind starting
    /// at 1.
    ///
    /// The next version is computed inline against the table:
    /// `COALESCE(MAX(version), 0) + 1`. Two registrations racing on
    /// the same kind can both compute the same next-version; the
    /// unique constraint on `(spec_kind, version)` rejects the loser
    /// with [`WorkflowLibraryError::DuplicateKind`]. Callers that
    /// expect contention should retry.
    pub async fn register(
        &self,
        kind: &str,
        workflow: &Workflow,
    ) -> Result<i32, WorkflowLibraryError> {
        let json = serde_json::to_value(workflow)?;
        let row_id = uuid::Uuid::new_v4().to_string();

        let result = sqlx::query_scalar::<_, i32>(
            "INSERT INTO workflow_library (id, spec_kind, version, workflow_json) \
             VALUES ($1, $2, \
                     COALESCE((SELECT MAX(version) FROM workflow_library WHERE spec_kind = $2), 0) + 1, \
                     $3) \
             RETURNING version",
        )
        .bind(&row_id)
        .bind(kind)
        .bind(&json)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(version) => Ok(version),
            Err(sqlx::Error::Database(db_err))
                if db_err.constraint() == Some(KIND_VERSION_UNIQUE_CONSTRAINT) =>
            {
                // The race-loser does not know which version it lost
                // — re-read the latest to report a useful error.
                let current = self.latest_version(kind).await.unwrap_or(0);
                Err(WorkflowLibraryError::DuplicateKind {
                    kind: kind.to_string(),
                    version: current,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Return the latest registered `Workflow` for `kind`, or
    /// `Ok(None)` if the kind has never been registered.
    ///
    /// This is the ADR 0016 compiler-facing surface: "library[kind]"
    /// resolves to a single active `Workflow`.
    pub async fn lookup(&self, kind: &str) -> Result<Option<Workflow>, WorkflowLibraryError> {
        self.latest(kind).await
    }

    /// Same as [`lookup`](Self::lookup) — return the workflow at the
    /// highest registered version for `kind`. Spelled out as
    /// `latest()` to make the "max version wins" semantics explicit
    /// at call sites that care about the version-pick mechanic
    /// rather than the catalog surface.
    pub async fn latest(&self, kind: &str) -> Result<Option<Workflow>, WorkflowLibraryError> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT workflow_json FROM workflow_library \
             WHERE spec_kind = $1 \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(kind)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((json,)) => Ok(Some(serde_json::from_value(json)?)),
            None => Ok(None),
        }
    }

    /// Helper for the `DuplicateKind` error path — read the current
    /// `MAX(version)` for a kind, returning `None` if no rows yet.
    async fn latest_version(&self, kind: &str) -> Option<i32> {
        sqlx::query_scalar::<_, Option<i32>>(
            "SELECT MAX(version) FROM workflow_library WHERE spec_kind = $1",
        )
        .bind(kind)
        .fetch_one(&self.pool)
        .await
        .ok()
        .flatten()
    }
}
