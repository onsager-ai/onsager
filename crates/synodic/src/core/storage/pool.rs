//! Connection pool factory — detects database backend from DATABASE_URL.

use anyhow::{Context, Result};

use super::Storage;

/// Create a storage backend from a DATABASE_URL string.
///
/// - `sqlite://path/to/db.sqlite` → SQLite
/// - `postgres://...` or `postgresql://...` → PostgreSQL
///
/// Runs pending migrations automatically on connect.
pub async fn create_storage(database_url: &str) -> Result<Box<dyn Storage>> {
    if database_url.starts_with("sqlite:") {
        #[cfg(feature = "sqlite")]
        {
            let store = super::sqlite::SqliteStorage::connect(database_url)
                .await
                .context("connecting to SQLite")?;
            store.migrate().await.context("running SQLite migrations")?;
            Ok(Box::new(store))
        }
        #[cfg(not(feature = "sqlite"))]
        {
            anyhow::bail!("SQLite support not compiled in — enable the `sqlite` feature");
        }
    } else if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        #[cfg(feature = "postgres")]
        {
            let store = super::postgres::PostgresStorage::connect(database_url)
                .await
                .context("connecting to PostgreSQL")?;
            store
                .migrate()
                .await
                .context("running PostgreSQL migrations")?;
            Ok(Box::new(store))
        }
        #[cfg(not(feature = "postgres"))]
        {
            anyhow::bail!("PostgreSQL support not compiled in — enable the `postgres` feature");
        }
    } else {
        anyhow::bail!(
            "unsupported DATABASE_URL scheme — expected sqlite:// or postgres://: {database_url}"
        );
    }
}

/// Resolve the DATABASE_URL from environment or default to a local SQLite file.
pub fn resolve_database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| default_sqlite_url())
}

/// Default SQLite path: `~/.synodic/synodic.db`
fn default_sqlite_url() -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    format!("sqlite://{home}/.synodic/synodic.db?mode=rwc")
}
