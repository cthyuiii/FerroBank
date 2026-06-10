//! Transfer service — owned by the Transfers module (Member 4).
//!
//! Demonstrates two layers of concurrency control:
//!
//! 1. **Application-level rate limiting** via `tokio::sync::Mutex` over an
//!    in-memory `HashMap`. Cheap, immediate, and prevents abusive bursts from
//!    a single account before they ever reach the database.
//!
//! 2. **Database-level row locks** via `SELECT ... FOR UPDATE` inside a single
//!    SQL transaction. Provides the strong correctness guarantee (no lost
//!    updates, no double spending) when multiple Actix workers process
//!    concurrent transfers against the same accounts.
//!
//! Together they answer the spec's call for "thread safety, transactional
//! consistency, Mutex locking, rollback mechanisms, and concurrent request
//! handling within Rust and Actix Web."

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rand::Rng;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::errors::AppError;
use crate::models::account::AccountStatus;
use crate::models::transfer::{Transfer, TransferStatus};
use crate::services::audit_service::AuditService;
use crate::services::telegram_service::OtpChannel;

// ── Tunables ─────────────────────────────────────────────────────────
const MAX_TRANSFERS_PER_WINDOW: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Public result of a successful `create` call. The handler shows the
/// plaintext OTP on the confirm page (a real bank would SMS it instead).
pub struct TransferCreated {
    pub transfer: Transfer,
    pub otp: String,
    /// `true` when the OTP was delivered out-of-band (Telegram). The handler
    /// then hides the on-screen code and tells the user to check their phone.
    pub otp_delivered: bool,
}

#[async_trait]
pub trait TransferService: Send + Sync {
    /// Step 1: validate, rate-limit, generate an OTP, insert a pending row.
    /// Money does **not** move yet.
    async fn create(
        &self,
        actor: i64,
        from_account_id: i64,
        to_account_id: i64,
        amount: Decimal,
        note: Option<String>,
    ) -> Result<TransferCreated, AppError>;

    /// Step 2: verify the OTP, move money inside one SQL transaction,
    /// audit the result.
    async fn confirm(&self, actor: i64, transfer_id: i64, otp: &str)
        -> Result<Transfer, AppError>;

    /// All transfers a user can see (either as sender or recipient).
    async fn history(&self, user_id: i64) -> Result<Vec<Transfer>, AppError>;
}

pub struct PgTransferService {
    // All state is private — handlers depend only on the `TransferService` trait,
    // never on these fields. This is the encapsulation boundary.
    db: PgPool,
    audit: Arc<dyn AuditService>,
    /// Out-of-band OTP delivery (Telegram), with on-screen fallback. Injected
    /// as a trait object so the engine never knows which impl it's using.
    otp_channel: Arc<dyn OtpChannel>,
    /// Per-account rolling list of recent attempt timestamps, gated by an
    /// async-aware Mutex. See module docs for why this lives alongside the
    /// database row locks.
    rate_limit: Mutex<HashMap<i64, Vec<Instant>>>,
}

impl PgTransferService {
    pub fn new(
        db: PgPool,
        audit: Arc<dyn AuditService>,
        otp_channel: Arc<dyn OtpChannel>,
    ) -> Self {
        Self {
            db,
            audit,
            otp_channel,
            rate_limit: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `Ok(())` if the account is under the per-minute limit; otherwise
    /// `AppError::Conflict`. Side effect: records this attempt's timestamp.
    async fn check_rate_limit(&self, account_id: i64) -> Result<(), AppError> {
        let now = Instant::now();
        let mut map = self.rate_limit.lock().await;
        let entry = map.entry(account_id).or_default();
        // Drop timestamps older than the window — keeps the vec bounded.
        entry.retain(|t| now.saturating_duration_since(*t) <= RATE_WINDOW);
        if entry.len() >= MAX_TRANSFERS_PER_WINDOW {
            return Err(AppError::Conflict(format!(
                "rate limit: more than {} transfers in {}s",
                MAX_TRANSFERS_PER_WINDOW,
                RATE_WINDOW.as_secs()
            )));
        }
        entry.push(now);
        Ok(())
    }
}

// ── Internal row used inside the confirm() transaction ──────────────
//
// We don't expose otp_hash / confirmed_at on the public Transfer struct —
// they're implementation details of the OTP simulation.
#[derive(sqlx::FromRow)]
struct PendingRow {
    id: i64,
    from_account_id: i64,
    to_account_id: i64,
    amount: Decimal,
    status: TransferStatus,
    note: Option<String>,
    otp_hash: Option<String>,
    created_at: DateTime<Utc>,
}

#[async_trait]
impl TransferService for PgTransferService {
    async fn create(
        &self,
        actor: i64,
        from_account_id: i64,
        to_account_id: i64,
        amount: Decimal,
        note: Option<String>,
    ) -> Result<TransferCreated, AppError> {
        // ── Cheap validations before we touch the DB ────────────────────
        if amount <= Decimal::ZERO {
            return Err(AppError::BadRequest("amount must be greater than zero".into()));
        }
        if from_account_id == to_account_id {
            return Err(AppError::BadRequest("cannot transfer to the same account".into()));
        }

        // No self-transfers at all: both accounts must belong to DIFFERENT
        // users (a DB trigger from migration 008 backs this up).
        let owners: Option<(i64, i64)> = sqlx::query_as(
            r#"
            SELECT fa.user_id, ta.user_id
            FROM accounts fa, accounts ta
            WHERE fa.id = $1 AND ta.id = $2
            "#,
        )
        .bind(from_account_id)
        .bind(to_account_id)
        .fetch_optional(&self.db)
        .await?;
        match owners {
            Some((f, t)) if f != t => {}
            Some(_) => {
                return Err(AppError::BadRequest(
                    "you cannot transfer to yourself — the recipient account also belongs to you".into(),
                ))
            }
            None => {
                return Err(AppError::NotFound(
                    "one of the accounts does not exist".into(),
                ))
            }
        }

        // ── Application-level concurrency control ──────────────────────
        // Gate on a `tokio::sync::Mutex` before we even open a DB connection.
        self.check_rate_limit(from_account_id).await?;

        // ── OTP generation ─────────────────────────────────────────────
        // 6-digit zero-padded code. Hashed with argon2 so the DB never holds
        // the plaintext (defends against an attacker who reads the DB but
        // not the live HTTPS response).
        let otp: String = {
            let mut rng = rand::thread_rng();
            format!("{:06}", rng.gen_range(0..1_000_000u32))
        };
        let salt = SaltString::generate(&mut OsRng);
        let otp_hash = Argon2::default()
            .hash_password(otp.as_bytes(), &salt)
            .map_err(AppError::from)?
            .to_string();

        // ── Insert pending row ─────────────────────────────────────────
        let transfer = sqlx::query_as::<_, Transfer>(
            r#"
            INSERT INTO transfers
                (from_account_id, to_account_id, amount, status, note, otp_hash)
            VALUES
                ($1, $2, $3, 'pending', $4, $5)
            RETURNING
                id, from_account_id, to_account_id, amount, status, note, status_reason, created_at
            "#,
        )
        .bind(from_account_id)
        .bind(to_account_id)
        .bind(amount)
        .bind(note.as_deref())
        .bind(&otp_hash)
        .fetch_one(&self.db)
        .await?;

        // ── Audit + dev-only log of the OTP ────────────────────────────
        self.audit
            .record(
                Some(actor),
                "transfer.created",
                json!({
                    "transfer_id": transfer.id,
                    "from_account_id": from_account_id,
                    "to_account_id": to_account_id,
                    "amount": amount.to_string(),
                }),
            )
            .await?;

        // Out-of-band delivery: send the code to the user's linked Telegram.
        // Falls back to on-screen display when the user isn't linked (or no
        // bot token is configured), so the demo always works.
        let otp_delivered = self.otp_channel.send_otp(actor, &otp).await;
        if !otp_delivered {
            tracing::info!(
                transfer_id = transfer.id,
                otp = %otp,
                "transfer created — OTP shown on screen (no Telegram linked)"
            );
        }

        Ok(TransferCreated {
            transfer,
            otp,
            otp_delivered,
        })
    }

    async fn confirm(
        &self,
        actor: i64,
        transfer_id: i64,
        otp: &str,
    ) -> Result<Transfer, AppError> {
        let mut tx = self.db.begin().await?;

        // (1) Lock the transfer row. Anything but 'pending' is a no-op.
        let pending: PendingRow = sqlx::query_as::<_, PendingRow>(
            r#"
            SELECT id, from_account_id, to_account_id, amount, status, note, otp_hash, created_at
            FROM transfers
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(transfer_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("transfer {transfer_id} not found")))?;

        if pending.status != TransferStatus::Pending {
            return Err(AppError::Conflict(format!(
                "transfer is already {}",
                pending.status.label().to_lowercase()
            )));
        }

        // (2) The actor must own the source account. Without this check, any
        // logged-in user who learned a transfer id could confirm it — or kill
        // it by burning OTP attempts. Checked BEFORE OTP verification so a
        // stranger's bad guesses can never flip the transfer to rejected.
        let owner: Option<(i64,)> =
            sqlx::query_as(r#"SELECT user_id FROM accounts WHERE id = $1"#)
                .bind(pending.from_account_id)
                .fetch_optional(&mut *tx)
                .await?;
        match owner {
            Some((user_id,)) if user_id == actor => {}
            // Dropping `tx` without commit rolls back; nothing was modified.
            _ => return Err(AppError::Forbidden),
        }

        // (3) Verify the OTP.
        let stored_hash = pending
            .otp_hash
            .as_deref()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("pending transfer missing OTP")))?;
        let parsed = PasswordHash::new(stored_hash)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("malformed OTP hash: {e}")))?;

        if Argon2::default()
            .verify_password(otp.as_bytes(), &parsed)
            .is_err()
        {
            // Reject and commit so the rejection is durable, audit afterwards.
            mark_rejected(&mut tx, transfer_id, "invalid one-time confirmation code").await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.otp_failed",
                    json!({ "transfer_id": transfer_id }),
                )
                .await?;
            return Err(AppError::BadRequest("invalid confirmation code".into()));
        }

        // (4) Lock both account rows. ORDER BY id eliminates deadlock potential
        //     when two concurrent transfers touch the same pair of accounts in
        //     opposite directions.
        let (low, high) = if pending.from_account_id < pending.to_account_id {
            (pending.from_account_id, pending.to_account_id)
        } else {
            (pending.to_account_id, pending.from_account_id)
        };
        let accounts: Vec<(i64, AccountStatus, Decimal)> = sqlx::query_as(
            r#"
            SELECT id, status, balance
            FROM accounts
            WHERE id IN ($1, $2)
            ORDER BY id
            FOR UPDATE
            "#,
        )
        .bind(low)
        .bind(high)
        .fetch_all(&mut *tx)
        .await?;

        if accounts.len() != 2 {
            return Err(AppError::Conflict(
                "one of the accounts no longer exists".into(),
            ));
        }

        // unwrap() is safe — we asserted len() == 2 just above.
        let from = accounts.iter().find(|a| a.0 == pending.from_account_id).unwrap();
        let to = accounts.iter().find(|a| a.0 == pending.to_account_id).unwrap();

        // (5) Business-rule re-checks under the locks.
        let reject_with = |reason: &str| -> AppError { AppError::Conflict(reason.into()) };

        if from.1 != AccountStatus::Active {
            let reason = format!("source account is {}", from.1.label().to_lowercase());
            mark_rejected(&mut tx, transfer_id, &reason).await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.rejected",
                    json!({ "transfer_id": transfer_id, "reason": "source not active" }),
                )
                .await?;
            return Err(reject_with(&format!(
                "source account is {}",
                from.1.label().to_lowercase()
            )));
        }
        if to.1 != AccountStatus::Active {
            let reason = format!("destination account is {}", to.1.label().to_lowercase());
            mark_rejected(&mut tx, transfer_id, &reason).await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.rejected",
                    json!({ "transfer_id": transfer_id, "reason": "destination not active" }),
                )
                .await?;
            return Err(reject_with(&format!(
                "destination account is {}",
                to.1.label().to_lowercase()
            )));
        }
        if from.2 < pending.amount {
            let reason = format!(
                "insufficient funds: balance ${} is less than ${}",
                from.2, pending.amount
            );
            mark_rejected(&mut tx, transfer_id, &reason).await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.rejected",
                    json!({
                        "transfer_id": transfer_id,
                        "reason": "insufficient funds",
                        "balance": from.2.to_string(),
                        "amount": pending.amount.to_string(),
                    }),
                )
                .await?;
            return Err(reject_with(&format!(
                "insufficient funds: balance ${} < ${}",
                from.2, pending.amount
            )));
        }

        // (6) Move money + finalize transfer, all inside the same transaction.
        //
        // We run the three writes through a helper and roll back EXPLICITLY if
        // any of them fails. (sqlx also rolls back automatically when a `tx` is
        // dropped without committing, but doing it explicitly makes the
        // atomicity guarantee visible: a partial debit can never be committed.)
        if let Err(e) = apply_money_move(
            &mut tx,
            pending.from_account_id,
            pending.to_account_id,
            pending.amount,
            transfer_id,
        )
        .await
        {
            tx.rollback().await?;
            tracing::warn!(transfer_id, error = %e, "transfer rolled back");
            return Err(e);
        }

        tx.commit().await?;

        // (7) Audit outside the transaction — the audit_log row is its own
        //     atomic write and we don't want it blocking the money move.
        self.audit
            .record(
                Some(actor),
                "transfer.completed",
                json!({
                    "transfer_id": transfer_id,
                    "from_account_id": pending.from_account_id,
                    "to_account_id": pending.to_account_id,
                    "amount": pending.amount.to_string(),
                }),
            )
            .await?;

        Ok(Transfer {
            id: pending.id,
            from_account_id: pending.from_account_id,
            to_account_id: pending.to_account_id,
            amount: pending.amount,
            status: TransferStatus::Completed,
            note: pending.note,
            status_reason: None,
            created_at: pending.created_at,
        })
    }

    async fn history(&self, user_id: i64) -> Result<Vec<Transfer>, AppError> {
        let rows = sqlx::query_as::<_, Transfer>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id, t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            WHERE t.from_account_id IN (SELECT id FROM accounts WHERE user_id = $1)
               OR t.to_account_id   IN (SELECT id FROM accounts WHERE user_id = $1)
            ORDER BY t.created_at DESC
            LIMIT 200
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}

/// The three writes that make up a completed transfer: debit the sender, credit
/// the recipient, and finalize the transfer row. Kept in one place so `confirm`
/// can wrap it in an explicit rollback-on-error.
async fn apply_money_move(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    from_account_id: i64,
    to_account_id: i64,
    amount: Decimal,
    transfer_id: i64,
) -> Result<(), AppError> {
    sqlx::query(r#"UPDATE accounts SET balance = balance - $1 WHERE id = $2"#)
        .bind(amount)
        .bind(from_account_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(r#"UPDATE accounts SET balance = balance + $1 WHERE id = $2"#)
        .bind(amount)
        .bind(to_account_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE transfers
        SET status = 'completed', confirmed_at = now(), otp_hash = NULL
        WHERE id = $1
        "#,
    )
    .bind(transfer_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Tiny helper to keep the rejection paths in `confirm()` readable. Records the
/// human-readable reason alongside the status so the UI can explain the failure.
async fn mark_rejected(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    transfer_id: i64,
    reason: &str,
) -> Result<(), AppError> {
    sqlx::query(r#"UPDATE transfers SET status = 'rejected', status_reason = $2 WHERE id = $1"#)
        .bind(transfer_id)
        .bind(reason)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
