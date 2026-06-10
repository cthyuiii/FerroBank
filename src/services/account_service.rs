//! Account service — owned by the Accounts module (Member 3).

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use chrono::{DateTime, Utc};

use crate::models::account::{Account, AccountStatus, AccountType};

#[async_trait]
pub trait AccountService: Send + Sync {
    /// Open an account. `approved = false` creates it as `pending` (a customer
    /// self-opening, which a teller/admin must then approve); `approved = true`
    /// creates it `active` immediately (used by staff/seed).
    async fn open_account(
        &self,
        user_id: i64,
        kind: AccountType,
        approved: bool,
    ) -> Result<Account, AppError>;
    /// Staff action: approve a pending account, making it active.
    async fn approve_account(&self, account_id: i64) -> Result<(), AppError>;
    async fn close_account(&self, account_id: i64) -> Result<(), AppError>;
    async fn freeze_account(&self, account_id: i64) -> Result<(), AppError>;
    /// Staff action: return a frozen account to active.
    async fn unfreeze_account(&self, account_id: i64) -> Result<(), AppError>;
    /// Staff action: apply a signed manual adjustment (credit if positive,
    /// debit if negative). Returns the new balance. Rejects a debit that would
    /// push the balance negative.
    async fn adjust_balance(&self, account_id: i64, delta: Decimal) -> Result<Decimal, AppError>;
    async fn get_balance(&self, account_id: i64) -> Result<Decimal, AppError>;
    async fn get_by_id(&self, account_id: i64) -> Result<Account, AppError>;
    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Account>, AppError>;

    /// Customer requests a transfer-limit change. Decreases apply at once;
    /// increases are held for the account's hold window (12 h default) before
    /// taking effect — a hijacked session can't instantly raise and drain.
    /// Returns the moment the new limit becomes effective.
    async fn request_limit_change(
        &self,
        account_id: i64,
        new_limit: Decimal,
    ) -> Result<DateTime<Utc>, AppError>;

    /// The most recent still-pending limit change for an account, if any.
    async fn pending_limit_change(
        &self,
        account_id: i64,
    ) -> Result<Option<(Decimal, DateTime<Utc>)>, AppError>;

    // ── Read-only methods exposed to the Admin Dashboard ─────────────
    async fn count_active(&self) -> Result<i64, AppError>;
    async fn total_deposits(&self) -> Result<Decimal, AppError>;
}

pub struct PgAccountService {
    db: PgPool,
}

impl PgAccountService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Generate a 10-digit account number. Collisions are vanishingly rare at
    /// this scale, but the UNIQUE constraint on `account_number` will catch any
    /// race and `open_account` retries up to 3 times.
    fn random_account_number() -> String {
        let mut rng = rand::thread_rng();
        let n: u64 = rng.gen_range(1_000_000_000..=9_999_999_999);
        n.to_string()
    }
}

#[async_trait]
impl AccountService for PgAccountService {
    async fn open_account(
        &self,
        user_id: i64,
        kind: AccountType,
        approved: bool,
    ) -> Result<Account, AppError> {
        let status = if approved {
            AccountStatus::Active
        } else {
            AccountStatus::Pending
        };

        // Retry on the (extremely unlikely) account-number collision.
        for attempt in 0..3 {
            let account_number = Self::random_account_number();

            let result = sqlx::query_as::<_, Account>(
                r#"
                INSERT INTO accounts (user_id, account_number, kind, status, balance)
                VALUES ($1, $2, $3, $4, 0)
                RETURNING id, user_id, account_number, kind, status, balance, transfer_limit, created_at
                "#,
            )
            .bind(user_id)
            .bind(&account_number)
            .bind(kind)
            .bind(status)
            .fetch_one(&self.db)
            .await;

            match result {
                Ok(account) => {
                    tracing::info!(
                        user_id,
                        account_id = account.id,
                        account_number = %account.account_number,
                        kind = ?account.kind,
                        status = ?account.status,
                        "opened account"
                    );
                    return Ok(account);
                }
                Err(sqlx::Error::Database(db_err))
                    if db_err.is_unique_violation() && attempt < 2 =>
                {
                    tracing::warn!(attempt, "account_number collision, retrying");
                    continue;
                }
                Err(e) => return Err(AppError::from(e)),
            }
        }
        Err(AppError::Internal(anyhow::anyhow!(
            "failed to open account after 3 attempts"
        )))
    }

    async fn approve_account(&self, account_id: i64) -> Result<(), AppError> {
        let rows = sqlx::query(
            r#"UPDATE accounts SET status = 'active' WHERE id = $1 AND status = 'pending'"#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(AppError::Conflict(
                "account is not pending approval".into(),
            ));
        }

        tracing::info!(account_id, "approved account");
        Ok(())
    }

    async fn close_account(&self, account_id: i64) -> Result<(), AppError> {
        // Refuse to close an account that still holds money.
        let account = self.get_by_id(account_id).await?;
        if account.status == AccountStatus::Closed {
            return Err(AppError::Conflict("account is already closed".into()));
        }
        if account.balance != Decimal::ZERO {
            return Err(AppError::Conflict(format!(
                "cannot close account with non-zero balance ({})",
                account.balance
            )));
        }

        sqlx::query(
            r#"UPDATE accounts SET status = 'closed' WHERE id = $1 AND status <> 'closed'"#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;

        tracing::info!(account_id, "closed account");
        Ok(())
    }

    async fn freeze_account(&self, account_id: i64) -> Result<(), AppError> {
        let rows = sqlx::query(
            r#"UPDATE accounts SET status = 'frozen' WHERE id = $1 AND status = 'active'"#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(AppError::Conflict(
                "account is not active and cannot be frozen".into(),
            ));
        }

        tracing::info!(account_id, "froze account");
        Ok(())
    }

    async fn unfreeze_account(&self, account_id: i64) -> Result<(), AppError> {
        let rows = sqlx::query(
            r#"UPDATE accounts SET status = 'active' WHERE id = $1 AND status = 'frozen'"#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(AppError::Conflict(
                "account is not frozen and cannot be unfrozen".into(),
            ));
        }

        tracing::info!(account_id, "unfroze account");
        Ok(())
    }

    async fn adjust_balance(&self, account_id: i64, delta: Decimal) -> Result<Decimal, AppError> {
        if delta == Decimal::ZERO {
            return Err(AppError::BadRequest("adjustment cannot be zero".into()));
        }

        let mut tx = self.db.begin().await?;

        let current: (Decimal,) =
            sqlx::query_as(r#"SELECT balance FROM accounts WHERE id = $1 FOR UPDATE"#)
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;

        let new_balance = current.0 + delta;
        if new_balance < Decimal::ZERO {
            return Err(AppError::Conflict(format!(
                "adjustment would make the balance negative (${} + ${})",
                current.0, delta
            )));
        }

        sqlx::query(r#"UPDATE accounts SET balance = $1 WHERE id = $2"#)
            .bind(new_balance)
            .bind(account_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        tracing::info!(account_id, %delta, %new_balance, "balance adjusted");
        Ok(new_balance)
    }

    async fn get_balance(&self, account_id: i64) -> Result<Decimal, AppError> {
        let row: (Decimal,) = sqlx::query_as(r#"SELECT balance FROM accounts WHERE id = $1"#)
            .bind(account_id)
            .fetch_optional(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;
        Ok(row.0)
    }

    async fn get_by_id(&self, account_id: i64) -> Result<Account, AppError> {
        sqlx::query_as::<_, Account>(
            r#"
            SELECT id, user_id, account_number, kind, status, balance, transfer_limit, created_at
            FROM accounts
            WHERE id = $1
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))
    }

    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Account>, AppError> {
        let rows = sqlx::query_as::<_, Account>(
            r#"
            SELECT id, user_id, account_number, kind, status, balance, transfer_limit, created_at
            FROM accounts
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn request_limit_change(
        &self,
        account_id: i64,
        new_limit: Decimal,
    ) -> Result<DateTime<Utc>, AppError> {
        if new_limit <= Decimal::ZERO {
            return Err(AppError::BadRequest("the limit must be positive".into()));
        }

        let (current, hold_seconds): (Decimal, i32) = sqlx::query_as(
            r#"SELECT transfer_limit, limit_hold_seconds FROM accounts WHERE id = $1"#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;

        if new_limit <= current {
            // Lowering the limit reduces risk — apply immediately.
            sqlx::query(r#"UPDATE accounts SET transfer_limit = $1 WHERE id = $2"#)
                .bind(new_limit)
                .bind(account_id)
                .execute(&self.db)
                .await?;
            tracing::info!(account_id, %new_limit, "limit lowered immediately");
            return Ok(Utc::now());
        }

        // Raising the limit: park it until the hold window matures. The
        // transfer engine applies matured rows lazily before enforcing.
        let (effective_at,): (DateTime<Utc>,) = sqlx::query_as(
            r#"
            INSERT INTO limit_changes (account_id, old_limit, new_limit, effective_at)
            VALUES ($1, $2, $3, now() + ($4 || ' seconds')::interval)
            RETURNING effective_at
            "#,
        )
        .bind(account_id)
        .bind(current)
        .bind(new_limit)
        .bind(hold_seconds.to_string())
        .fetch_one(&self.db)
        .await?;

        tracing::info!(account_id, %new_limit, %effective_at, "limit increase parked");
        Ok(effective_at)
    }

    async fn pending_limit_change(
        &self,
        account_id: i64,
    ) -> Result<Option<(Decimal, DateTime<Utc>)>, AppError> {
        let row: Option<(Decimal, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT new_limit, effective_at FROM limit_changes
            WHERE account_id = $1 AND applied_at IS NULL AND effective_at > now()
            ORDER BY effective_at DESC LIMIT 1
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row)
    }

    // ── Admin Dashboard hooks ────────────────────────────────────────

    async fn count_active(&self) -> Result<i64, AppError> {
        let row: (i64,) =
            sqlx::query_as(r#"SELECT COUNT(*)::BIGINT FROM accounts WHERE status = 'active'"#)
                .fetch_one(&self.db)
                .await?;
        Ok(row.0)
    }

    async fn total_deposits(&self) -> Result<Decimal, AppError> {
        let row: (Option<Decimal>,) =
            sqlx::query_as(r#"SELECT SUM(balance) FROM accounts WHERE status = 'active'"#)
                .fetch_one(&self.db)
                .await?;
        Ok(row.0.unwrap_or(Decimal::ZERO))
    }
}
