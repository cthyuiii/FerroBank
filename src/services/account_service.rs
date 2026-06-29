//! Account service

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use chrono::{DateTime, Utc};

use crate::models::account::{Account, AccountStatus, AccountType};
use std::sync::Arc;

use crate::models::user::Role;
use crate::services::audit_service::notify;
use crate::services::telegram_service::OtpChannel;

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
    /// taking effect - a hijacked session can't instantly raise and drain.
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

    /// Staff balance adjustment with dual control: |delta| <= $1,000 applies
    /// immediately (returns `Some(new_balance)`); anything larger is parked
    /// until a member of the OTHER staff role approves it (returns `None`).
    async fn request_adjustment(
        &self,
        account_id: i64,
        delta: Decimal,
        requester: i64,
        requester_role: Role,
    ) -> Result<Option<Decimal>, AppError>;

    /// Second pair of eyes: approve a parked adjustment. The approver's role
    /// must differ from the requester's. Returns the new balance.
    async fn approve_adjustment(
        &self,
        request_id: i64,
        approver: i64,
        approver_role: Role,
    ) -> Result<Decimal, AppError>;

    /// Adjustments still waiting for their second approval.
    async fn pending_adjustments(&self) -> Result<Vec<AdjustmentRow>, AppError>;

    // ── Read-only methods exposed to the Admin Dashboard ─────────────
    async fn count_active(&self) -> Result<i64, AppError>;
    async fn total_deposits(&self) -> Result<Decimal, AppError>;
}

/// One row of the pending-adjustments queue on the staff accounts page.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AdjustmentRow {
    pub id: i64,
    pub account_id: i64,
    pub account_number: String,
    pub delta: Decimal,
    pub requested_by_name: String,
    pub requested_role: Role,
    pub requested_at: DateTime<Utc>,
}

pub struct PgAccountService {
    db: PgPool,
    /// Telegram delivery for account lifecycle events (toasts always fire).
    otp_channel: Arc<dyn OtpChannel>,
}

impl PgAccountService {
    pub fn new(db: PgPool, otp_channel: Arc<dyn OtpChannel>) -> Self {
        Self { db, otp_channel }
    }

    /// Promote any limit change whose hold window has matured. Called on
    /// every account read so the page always shows the live limit (the
    /// transfer engine does the same before enforcing).
    async fn apply_matured_limit_changes(&self, account_id: i64) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE accounts a SET transfer_limit = lc.new_limit
            FROM (
                SELECT DISTINCT ON (account_id) account_id, new_limit
                FROM limit_changes
                WHERE account_id = $1 AND applied_at IS NULL AND effective_at <= now()
                ORDER BY account_id, effective_at DESC
            ) lc
            WHERE a.id = lc.account_id
            "#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;
        sqlx::query(
            r#"
            UPDATE limit_changes SET applied_at = now()
            WHERE account_id = $1 AND applied_at IS NULL AND effective_at <= now()
            "#,
        )
        .bind(account_id)
        .execute(&self.db)
        .await?;
        Ok(())
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
        let row: Option<(i64, AccountType)> = sqlx::query_as(
            r#"
            UPDATE accounts SET status = 'active'
            WHERE id = $1 AND status = 'pending'
            RETURNING user_id, kind
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?;

        let Some((owner, kind)) = row else {
            return Err(AppError::Conflict("account is not pending approval".into()));
        };

        let msg = format!(
            "Your new {} account was approved and is now active.",
            kind.label().to_lowercase()
        );
        notify(&self.db, owner, &msg).await;
        self.otp_channel.send_note(owner, &msg).await;

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
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            UPDATE accounts SET status = 'frozen'
            WHERE id = $1 AND status = 'active'
            RETURNING user_id
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?;

        let Some((owner,)) = row else {
            return Err(AppError::Conflict(
                "account is not active and cannot be frozen".into(),
            ));
        };

        let msg = "One of your accounts was frozen by the bank. Transfers from it are blocked - contact support if this is unexpected.";
        notify(&self.db, owner, msg).await;
        self.otp_channel.send_note(owner, msg).await;

        tracing::info!(account_id, "froze account");
        Ok(())
    }

    async fn unfreeze_account(&self, account_id: i64) -> Result<(), AppError> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            UPDATE accounts SET status = 'active'
            WHERE id = $1 AND status = 'frozen'
            RETURNING user_id
            "#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?;

        let Some((owner,)) = row else {
            return Err(AppError::Conflict(
                "account is not frozen and cannot be unfrozen".into(),
            ));
        };

        let msg = "Your account was unfrozen - you can transact again.";
        notify(&self.db, owner, msg).await;
        self.otp_channel.send_note(owner, msg).await;

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
        // Matured limit increases must be visible the moment a user looks at
        // the account, not only when the transfer engine next runs.
        self.apply_matured_limit_changes(account_id).await?;
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

        let (current, hold_seconds, owner): (Decimal, i32, i64) = sqlx::query_as(
            r#"SELECT transfer_limit, limit_hold_seconds, user_id FROM accounts WHERE id = $1"#,
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;

        if new_limit <= current {
            // Lowering the limit reduces risk - apply immediately.
            sqlx::query(r#"UPDATE accounts SET transfer_limit = $1 WHERE id = $2"#)
                .bind(new_limit)
                .bind(account_id)
                .execute(&self.db)
                .await?;
            notify(
                &self.db,
                owner,
                &format!("Your per-transfer limit was lowered to ${new_limit}, effective immediately."),
            )
            .await;
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

        notify(
            &self.db,
            owner,
            &format!("Limit increase to ${new_limit} accepted. For your security it is held before taking effect - the account page shows the exact time in your local timezone."),
        )
        .await;
        tracing::info!(account_id, %new_limit, %effective_at, "limit increase parked");
        Ok(effective_at)
    }

    async fn request_adjustment(
        &self,
        account_id: i64,
        delta: Decimal,
        requester: i64,
        requester_role: Role,
    ) -> Result<Option<Decimal>, AppError> {
        if requester_role == Role::Customer {
            return Err(AppError::Forbidden);
        }
        if delta == Decimal::ZERO {
            return Err(AppError::BadRequest("adjustment cannot be zero".into()));
        }

        // Small adjustments apply on the spot.
        if delta.abs() <= Decimal::from(1_000) {
            let new_balance = self.adjust_balance(account_id, delta).await?;
            return Ok(Some(new_balance));
        }

        // Large adjustments wait for the other role's approval.
        sqlx::query(
            r#"
            INSERT INTO adjustment_requests (account_id, delta, requested_by, requested_role)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(account_id)
        .bind(delta)
        .bind(requester)
        .bind(requester_role)
        .execute(&self.db)
        .await?;
        tracing::info!(account_id, %delta, "large adjustment parked for dual approval");
        Ok(None)
    }

    async fn approve_adjustment(
        &self,
        request_id: i64,
        approver: i64,
        approver_role: Role,
    ) -> Result<Decimal, AppError> {
        if approver_role == Role::Customer {
            return Err(AppError::Forbidden);
        }

        let mut tx = self.db.begin().await?;
        let req: Option<(i64, Decimal, i64, Role)> = sqlx::query_as(
            r#"
            SELECT account_id, delta, requested_by, requested_role
            FROM adjustment_requests
            WHERE id = $1 AND approved_at IS NULL
            FOR UPDATE
            "#,
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((account_id, delta, requested_by, requested_role)) = req else {
            return Err(AppError::Conflict(
                "this adjustment request no longer exists or was already approved".into(),
            ));
        };
        if approver == requested_by || approver_role == requested_role {
            return Err(AppError::Conflict(
                "dual control: a member of the OTHER staff role must approve this adjustment".into(),
            ));
        }

        // Apply under lock, refusing a negative result.
        let current: (Decimal,) =
            sqlx::query_as(r#"SELECT balance FROM accounts WHERE id = $1 FOR UPDATE"#)
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;
        let new_balance = current.0 + delta;
        if new_balance < Decimal::ZERO {
            return Err(AppError::Conflict(
                "applying this adjustment would make the balance negative".into(),
            ));
        }
        sqlx::query(r#"UPDATE accounts SET balance = $1 WHERE id = $2"#)
            .bind(new_balance)
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"UPDATE adjustment_requests SET approved_by = $2, approved_at = now() WHERE id = $1"#,
        )
        .bind(request_id)
        .bind(approver)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        tracing::info!(request_id, account_id, %delta, %new_balance, "dual-approved adjustment applied");
        Ok(new_balance)
    }

    async fn pending_adjustments(&self) -> Result<Vec<AdjustmentRow>, AppError> {
        let rows = sqlx::query_as::<_, AdjustmentRow>(
            r#"
            SELECT r.id, r.account_id, a.account_number, r.delta,
                   u.full_name AS requested_by_name, r.requested_role, r.requested_at
            FROM adjustment_requests r
            JOIN accounts a ON a.id = r.account_id
            JOIN users    u ON u.id = r.requested_by
            WHERE r.approved_at IS NULL
            ORDER BY r.requested_at ASC
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
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
