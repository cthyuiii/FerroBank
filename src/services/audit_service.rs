//! Audit service — owned by the Transfers module (Member 4).
//!
//! Append-only log of important financial and security events. Other modules
//! (transfers, loans, admin actions) call `record(...)` after every state change
//! that matters for compliance.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
