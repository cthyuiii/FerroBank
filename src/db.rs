//! Postgres connection pool construction.

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Build a `PgPool` against the given database URL.
///
/// Uses sensible defaults for a classroom workload. Tune `max_connections`
/// in production based on observed load.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(60 * 5))
        .connect(database_url)
        .await?;

    Ok(pool)
}
