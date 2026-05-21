//! Account service — owned by the Accounts module (Member 3).

use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::account::{Account, AccountType};

#[async_trait]
pub trait AccountService: Send + Sync {
    async fn open_account(&self, user_id: i64, kind: AccountType) -> Result<Account, AppError>;
    async fn close_account(&self, account_id: i64) -> Result<(), AppError>;
    async fn freeze_account(&self, account_id: i64) -> Result<(), AppError>;
    async fn get_balance(&self, account_id: i64) -> Result<Decimal, AppError>;
    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Account>, AppError>;

    // ── Read-only methods exposed to the Admin Dashboard ─────────────
    // Keep these cheap (single SELECT, indexed columns). The dashboard calls
    // them on every load.
    async fn count_active(&self) -> Result<i64, AppError>;
    async fn total_deposits(&self) -> Result<Decimal, AppError>;
}

pub struct PgAccountService {
    pub db: PgPool,
}

impl PgAccountService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AccountService for PgAccountService {
    async fn open_account(&self, _user_id: i64, _kind: AccountType) -> Result<Account, AppError> {
        // TODO(Member 3):
        //   - Generate a unique 10-digit account number.
        //   - INSERT INTO accounts (user_id, account_number, kind, status, balance)
        //     VALUES ($1, $2, $3, 'active', 0) RETURNING *.
        todo!("AccountService::open_account")
    }

    async fn close_account(&self, _account_id: i64) -> Result<(), AppError> {
        // TODO(Member 3):
        //   - Reject if balance != 0 (AppError::Conflict).
        //   - UPDATE accounts SET status = 'closed' WHERE id = $1.
        todo!("AccountService::close_account")
    }

    async fn freeze_account(&self, _account_id: i64) -> Result<(), AppError> {
        // TODO(Member 3): UPDATE accounts SET status = 'frozen' WHERE id = $1.
        todo!("AccountService::freeze_account")
    }

    async fn get_balance(&self, _account_id: i64) -> Result<Decimal, AppError> {
        // TODO(Member 3): SELECT balance FROM accounts WHERE id = $1.
        todo!("AccountService::get_balance")
    }

    async fn list_for_user(&self, _user_id: i64) -> Result<Vec<Account>, AppError> {
        // TODO(Member 3): SELECT * FROM accounts WHERE user_id = $1 ORDER BY created_at DESC.
        todo!("AccountService::list_for_user")
    }

    // ── Admin Dashboard hooks ────────────────────────────────────────
    // Return safe zero-values until Member 3 fills these in, so the
    // Platform Lead's dashboard renders without panicking.

    async fn count_active(&self) -> Result<i64, AppError> {
        Ok(0)
    }

    async fn total_deposits(&self) -> Result<Decimal, AppError> {
        Ok(Decimal::ZERO)
    }
}
