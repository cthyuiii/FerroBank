//! Shows the graceful-shutdown snapshot: `snapshot_system_state` appends a
//! `system.snapshot` row to the audit log capturing the bank's whole state.
//! `main.rs` calls the same function after the HTTP server shuts down.
//!
//! Needs a live Postgres (skips itself when DATABASE_URL is unset):
//!
//!     cargo test --test shutdown_snapshot -- --nocapture
//!
//! Inspect the rows by hand afterwards:
//!
//!     psql -U ferrobank -d ferrobank -c \
//!       "SELECT id, created_at, jsonb_pretty(payload) FROM audit_log
//!        WHERE event = 'system.snapshot' ORDER BY id DESC LIMIT 3;"

use ferrobank::services::audit_service::snapshot_system_state;

#[tokio::test]
async fn snapshot_appends_state_row_to_audit_log() {
    dotenvy::dotenv().ok();
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set (start Postgres and re-run)");
        return;
    };

    let pool = ferrobank::db::connect(&url).await.expect("db connect");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrations");

    let mut conn = pool.acquire().await.expect("acquire connection");
    snapshot_system_state(&mut conn).await.expect("snapshot must succeed");
    drop(conn);

    // The snapshot must be the newest audit row, with every expected metric.
    let (event, payload): (String, serde_json::Value) =
        sqlx::query_as(r#"SELECT event, payload FROM audit_log ORDER BY id DESC LIMIT 1"#)
            .fetch_one(&pool)
            .await
            .expect("read back audit row");

    assert_eq!(event, "system.snapshot");
    for key in [
        "accounts",
        "active_accounts",
        "total_deposits",
        "transfers",
        "completed_transfers",
        "loans",
        "repayments",
    ] {
        assert!(
            payload.get(key).is_some(),
            "snapshot payload missing `{key}`: {payload}"
        );
    }

    println!("snapshot log row: {payload}");
}
