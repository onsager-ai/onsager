//! Boot-time registration into the spine `artifact_adapters` catalog.
//!
//! Migration 004 created the `artifact_adapters` table but left it
//! empty — that's the "producer with no consumer" drift this spec
//! closes. Every Onsager binary that links `onsager-github` should
//! call [`register`] once at startup to upsert the GitHub adapter row.
//!
//! The capability set is encoded in `config` (a JSONB column) so
//! consumers can discover what kinds + intents this adapter speaks
//! without hard-coding the answer. Format is intentionally loose
//! pending #150's registry manifest, which will own the schema.

use serde_json::json;
use sqlx::{AnyPool, PgPool};

/// Stable adapter identifier. Matches the `external_ref.adapter` field
/// already in use on artifact rows for GitHub-sourced refs.
pub const ADAPTER_ID: &str = "github";

/// Current revision of the adapter capability set. Bump on every
/// schema-meaningful change to `config`; downstream consumers can
/// `WHERE revision >= N` to opt into newer fields.
pub const ADAPTER_REVISION: i32 = 1;

fn config_blob() -> serde_json::Value {
    json!({
        "kinds": ["pull_request", "issue", "check_run"],
        "intents": [],
        "version": ADAPTER_REVISION,
    })
}

/// Upsert the GitHub adapter row using a `PgPool`. Idempotent —
/// safe to call on every boot. The `'default'` workspace covers
/// single-tenant deployments that haven't migrated to per-workspace
/// rows yet.
///
/// Takes a `&PgPool` rather than the spine `EventStore` because the
/// store doesn't expose its pool publicly today; the underlying
/// schema lives in the spine migrations regardless.
pub async fn register(pool: &PgPool, workspace_id: &str) -> sqlx::Result<()> {
    let config = config_blob();
    sqlx::query(
        r#"
        INSERT INTO artifact_adapters (adapter_id, workspace_id, revision, status, config)
        VALUES ($1, $2, $3, 'approved', $4)
        ON CONFLICT (workspace_id, adapter_id) DO UPDATE
            SET revision = EXCLUDED.revision,
                config   = EXCLUDED.config,
                updated_at = NOW()
        "#,
    )
    .bind(ADAPTER_ID)
    .bind(workspace_id)
    .bind(ADAPTER_REVISION)
    .bind(&config)
    .execute(pool)
    .await?;

    log_registered(workspace_id);
    Ok(())
}

/// Same as [`register`] for an `AnyPool`. Stiglab uses sqlx's runtime-
/// polymorphic pool so it gets its own entry point; the SQL is
/// identical (postgres-style placeholders work under `Any` driver).
/// `Any` doesn't accept `JSONB` directly, so the config is passed as
/// a JSON string and cast in SQL.
pub async fn register_any(pool: &AnyPool, workspace_id: &str) -> sqlx::Result<()> {
    let config = config_blob().to_string();
    sqlx::query(
        r#"
        INSERT INTO artifact_adapters (adapter_id, workspace_id, revision, status, config)
        VALUES ($1, $2, $3, 'approved', CAST($4 AS JSONB))
        ON CONFLICT (workspace_id, adapter_id) DO UPDATE
            SET revision = EXCLUDED.revision,
                config   = EXCLUDED.config,
                updated_at = NOW()
        "#,
    )
    .bind(ADAPTER_ID)
    .bind(workspace_id)
    .bind(ADAPTER_REVISION)
    .bind(config)
    .execute(pool)
    .await?;

    log_registered(workspace_id);
    Ok(())
}

fn log_registered(workspace_id: &str) {
    tracing::info!(
        adapter = ADAPTER_ID,
        workspace_id = workspace_id,
        revision = ADAPTER_REVISION,
        "registered github adapter"
    );
}
