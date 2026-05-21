//! Transfer service — owned by the Transfers module (Member 4).
//!
//! THIS IS THE TECHNICAL CENTERPIECE OF THE PROJECT. The grader will look for:
//!   1. Single SQL transaction wrapping every money move.
//!   2. `SELECT ... FOR UPDATE` row locks on both account rows before reading balances.
//!   3. Validation: positive amount, both accounts exist, not frozen/closed,
//!      not the same account, sufficient balance.
//!   4. An audit log row written for every attempt (success OR rejection).
//!   5. OTP simulation: generate a 6-digit code, store it hashed, verify on confirm.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::transfer::Transfer;
use crate::services::audit_service::AuditService;

#[async_trait]
pub trait TransferService: Send + Sync {
    /// Step 1 of a transfer: create a pending row and an OTP. Money has NOT moved yet.
    async fn create(
        &self,
        from_account_id: i64,
        to_account_id: i64,
        amount: Decimal,
        note: Option<String>,
    ) -> Result<Transfer, AppError>;

    /// Step 2 of a transfer: user submits the OTP, money is moved inside a single SQL txn.
    async fn confirm(&self, transfer_id: i64, otp: &str) -> Result<Transfer, AppError>;

    /// All transfers visible to a particular user (either sender or recipient).
    async fn history(&self, user_id: i64) -> Result<Vec<Transfer>, AppError>;

    // ── Admin Dashboard hooks ────────────────────────────────────────
    async fn recent(&self, limit: i64) -> Result<Vec<Transfer>, AppError>;
    async fn flagged(&self) -> Result<Vec<Transfer>, AppError>;
}

pub struct PgTransferService {
    pub db: PgPool,
    pub audit: Arc<dyn AuditService>,
}

impl PgTransferService {
    pub fn new(db: PgPool, audit: Arc<dyn AuditService>) -> Self {
        Self { db, audit }
    }
}

#[async_trait]
impl TransferService for PgTransferService {
    async fn create(
        &self,
        _from_account_id: i64,
        _to_account_id: i64,
        _amount: Decimal,
        _note: Option<String>,
    ) -> Result<Transfer, AppError> {
        // TODO(Member 4):
        //   1. Validate amount > 0, from != to.
        //   2. Generate 6-digit OTP, hash with argon2, store in transfers.otp_hash.
        //   3. INSERT a Transfer row with status = 'pending'.
        //   4. Record audit event "transfer.created".
        //   5. (In real life you'd SMS the OTP. Here, write it to the tracing log so
        //      the grader can see it in the terminal.)
        todo!("TransferService::create")
    }

    async fn confirm(&self, _transfer_id: i64, _otp: &str) -> Result<Transfer, AppError> {
        // TODO(Member 4) — THE BIG ONE:
        //
        //   let mut tx = self.db.begin().await?;
        //
        //   1. SELECT * FROM transfers WHERE id = $1 FOR UPDATE  (must be pending).
        //   2. Verify OTP against stored hash.
        //   3. SELECT * FROM accounts WHERE id IN ($from, $to) ORDER BY id FOR UPDATE.
        //      (Order by id to avoid deadlocks under concurrency.)
        //   4. Re-check: both accounts active, balance >= amount.
        //   5. UPDATE accounts SET balance = balance - amount WHERE id = $from.
        //   6. UPDATE accounts SET balance = balance + amount WHERE id = $to.
        //   7. UPDATE transfers SET status = 'completed' WHERE id = $transfer_id.
        //   8. self.audit.record(Some(actor), "transfer.completed", json!({...})).
        //   9. tx.commit().
        //
        //   On any validation failure: set status = 'rejected', record audit event,
        //   commit the txn (so the rejection is durable), then return AppError::Conflict.
        todo!("TransferService::confirm")
    }

    async fn history(&self, _user_id: i64) -> Result<Vec<Transfer>, AppError> {
        // TODO(Member 4): SELECT transfers JOIN accounts WHERE accounts.user_id = $1.
        todo!("TransferService::history")
    }

    async fn recent(&self, _limit: i64) -> Result<Vec<Transfer>, AppError> {
        Ok(vec![])
    }

    async fn flagged(&self) -> Result<Vec<Transfer>, AppError> {
        Ok(vec![])
    }
}
