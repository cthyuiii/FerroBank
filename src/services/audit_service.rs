//! Audit service - owned by the Transfers module (Member 4).
//!
//! Append-only log of important financial and security events. Other modules
//! (transfers, loans, admin actions) call `record(...)` after every state change
//! that matters for compliance.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AuditEntry {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    /// Dotted event name, e.g. `"transfer.completed"`, `"loan.approved"`, `"auth.login"`.
    pub event: String,
    /// Free-form structured payload for the event.
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait AuditService: Send + Sync {
    async fn record(
        &self,
        actor: Option<i64>,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), AppError>;

    async fn recent(&self, limit: i64) -> Result<Vec<AuditEntry>, AppError>;
}

pub struct PgAuditService {
    db: PgPool,
}

impl PgAuditService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AuditService for PgAuditService {
    async fn record(
        &self,
        actor: Option<i64>,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO audit_log (actor_user_id, event, payload)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(actor)
        .bind(event)
        .bind(&payload)
        .execute(&self.db)
        .await?;

        // Mirror to the structured log so it shows up in the terminal during dev
        // and ships to whatever sink tracing is wired to in prod.
        tracing::info!(actor = ?actor, event = %event, payload = %payload, "audit");

        Ok(())
    }

    async fn recent(&self, limit: i64) -> Result<Vec<AuditEntry>, AppError> {
        let rows = sqlx::query_as::<_, AuditEntry>(
            r#"
            SELECT id, actor_user_id, event, payload, created_at
            FROM audit_log
            ORDER BY created_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}

// ── System snapshot ──────────────────────────────────────────────────

/// Whole-bank state snapshot, appended to the audit log as `system.snapshot`.
///
/// `main` calls this after the HTTP server finishes its graceful shutdown
/// (Ctrl-C / SIGTERM / `docker compose stop`), so the last audit row always
/// records the state the server last saw. A *hard* crash (kill -9, power
/// loss) can't run anything - but no data is lost there either: every
/// committed transaction is already durable via PostgreSQL's WAL. This row is
/// a forensic/ops marker, not a recovery mechanism.
///
/// A standalone function (not a trait method) so it can run after the
/// service `Arc`s are dropped, and is trivially callable from tests.
///
/// Takes a plain `PgConnection` rather than the pool: at shutdown the worker
/// pool's connections die with the workers, so acquiring from the shared pool
/// can time out. `main` opens one dedicated connection for this final write.
pub async fn snapshot_system_state(conn: &mut sqlx::PgConnection) -> Result<(), AppError> {
    let (accounts, active_accounts, total_deposits, transfers, completed, loans, repayments): (
        i64,
        i64,
        Option<Decimal>,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        r#"
        SELECT
            (SELECT COUNT(*)::BIGINT FROM accounts),
            (SELECT COUNT(*)::BIGINT FROM accounts  WHERE status = 'active'),
            (SELECT SUM(balance)     FROM accounts  WHERE status = 'active'),
            (SELECT COUNT(*)::BIGINT FROM transfers),
            (SELECT COUNT(*)::BIGINT FROM transfers WHERE status = 'completed'),
            (SELECT COUNT(*)::BIGINT FROM loans),
            (SELECT COUNT(*)::BIGINT FROM repayments)
        "#,
    )
    .fetch_one(&mut *conn)
    .await?;

    sqlx::query(
        r#"INSERT INTO audit_log (actor_user_id, event, payload) VALUES (NULL, 'system.snapshot', $1)"#,
    )
    .bind(serde_json::json!({
        "accounts": accounts,
        "active_accounts": active_accounts,
        "total_deposits": total_deposits.unwrap_or_default().to_string(),
        "transfers": transfers,
        "completed_transfers": completed,
        "loans": loans,
        "repayments": repayments,
    }))
    .execute(&mut *conn)
    .await?;

    tracing::info!(
        accounts,
        transfers,
        loans,
        "system.snapshot written to audit_log"
    );
    Ok(())
}

// ── User notifications ───────────────────────────────────────────────
// Lightweight per-user messages surfaced as browser toasts (layout.html
// polls /notifications) and mirrored to Telegram when the user is linked.

/// Queue a notification for a user. Failures are logged, never fatal - a
/// missed toast must not break a money movement.
pub async fn notify(db: &PgPool, user_id: i64, message: &str) {
    if let Err(e) = sqlx::query(r#"INSERT INTO notifications (user_id, message) VALUES ($1, $2)"#)
        .bind(user_id)
        .bind(message)
        .execute(db)
        .await
    {
        tracing::warn!(user_id, error = %e, "failed to queue notification");
    }
}

/// Fetch-and-mark: returns all unseen notifications for the user and stamps
/// them seen in the same statement, so each toast fires exactly once.
pub async fn take_unseen(db: &PgPool, user_id: i64) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        r#"
        UPDATE notifications SET seen_at = now()
        WHERE user_id = $1 AND seen_at IS NULL
        RETURNING message
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}
