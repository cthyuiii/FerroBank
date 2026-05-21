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
    pub db: PgPool,
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
        _actor: Option<i64>,
        _event: &str,
        _payload: serde_json::Value,
    ) -> Result<(), AppError> {
        // Stubbed as a noop until Member 4 wires the audit_log table.
        // INSERT INTO audit_log (actor_user_id, event, payload) VALUES ($1, $2, $3).
        Ok(())
    }

    async fn recent(&self, _limit: i64) -> Result<Vec<AuditEntry>, AppError> {
        Ok(vec![])
    }
}
