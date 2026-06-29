//! Transfer service
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
use crate::services::audit_service::{notify, AuditService};
use crate::services::telegram_service::OtpChannel;

// ── Tunables ─────────────────────────────────────────────────────────
const MAX_TRANSFERS_PER_WINDOW: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Public result of a successful `create` call. The code is delivered via the
/// injected OtpChannel; the plaintext is only surfaced on screen when no
/// out-of-band channel is configured.
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

    /// A transfer, visible only to the owner of its source account.
    async fn get_for_owner(&self, actor: i64, transfer_id: i64) -> Result<Transfer, AppError>;

    /// Customer submits (or updates) the purpose + identity claim for a held
    /// transfer, so staff can review it.
    async fn submit_review(
        &self,
        actor: i64,
        transfer_id: i64,
        purpose: &str,
        nric: &str,
    ) -> Result<(), AppError>;

    /// Staff queue: every on-hold transfer with its review request (if any)
    /// and the sender's NRIC on file for identity comparison.
    async fn list_held(&self) -> Result<Vec<HeldRow>, AppError>;

    /// Staff release: execute the held money move (same locks as confirm).
    async fn release(&self, reviewer: i64, transfer_id: i64) -> Result<(), AppError>;

    /// Staff denial: reject the held transfer with a reason.
    async fn deny(&self, reviewer: i64, transfer_id: i64, reason: &str) -> Result<(), AppError>;
}

/// One row of the staff review queue.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HeldRow {
    pub id: i64,
    pub amount: Decimal,
    pub status_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub from_number: String,
    pub to_number: String,
    pub from_user_id: i64,
    pub from_owner: String,
    /// NRIC stored on the sender's profile (staff compare against the claim).
    pub nric_on_file: Option<String>,
    pub to_owner: String,
    /// Review request fields - `None` until the customer submits one.
    pub purpose: Option<String>,
    pub nric_claimed: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
}

pub struct PgTransferService {
    // All state is private - handlers depend only on the `TransferService` trait,
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

    /// Hijack heuristic: 3+ attempts to send more than the balance within
    /// 24h looks like an attacker probing a stolen session, so the account
    /// freezes automatically and the owner is told on every channel.
    async fn check_overdraft_freeze(&self, actor: i64, account_id: i64) -> Result<(), AppError> {
        let (overdraft_attempts,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM transfers
            WHERE from_account_id = $1 AND status = 'rejected'
              AND status_reason LIKE 'insufficient funds%'
              AND created_at > now() - interval '24 hours'
            "#,
        )
        .bind(account_id)
        .fetch_one(&self.db)
        .await?;
        if overdraft_attempts >= 3 {
            let frozen = sqlx::query(
                r#"UPDATE accounts SET status = 'frozen' WHERE id = $1 AND status = 'active'"#,
            )
            .bind(account_id)
            .execute(&self.db)
            .await?;
            if frozen.rows_affected() > 0 {
                self.audit
                    .record(
                        Some(actor),
                        "account.frozen.suspected_hijack",
                        json!({ "account_id": account_id, "overdraft_attempts": overdraft_attempts }),
                    )
                    .await?;
                let msg = "Your account was frozen after repeated attempts to transfer more than its balance. Contact the bank to unfreeze it.";
                notify(&self.db, actor, msg).await;
                self.otp_channel.send_note(actor, msg).await;
            }
        }
        Ok(())
    }

    /// Returns `Ok(())` if the account is under the per-minute limit; otherwise
    /// `AppError::Conflict`. Side effect: records this attempt's timestamp.
    async fn check_rate_limit(&self, account_id: i64) -> Result<(), AppError> {
        let now = Instant::now();
        let mut map = self.rate_limit.lock().await;
        let entry = map.entry(account_id).or_default();
        // Drop timestamps older than the window - keeps the vec bounded.
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
// We don't expose otp_hash / confirmed_at on the public Transfer struct -
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
    otp_attempts: i32,
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
                    "you cannot transfer to yourself - the recipient account also belongs to you".into(),
                ))
            }
            None => {
                return Err(AppError::NotFound(
                    "one of the accounts does not exist".into(),
                ))
            }
        }

        // ── Per-transfer limit ──────────────────────────────────────────
        // First promote any limit change whose hold window has matured (lazy
        // application - no background job needed), then enforce the limit.
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
        .bind(from_account_id)
        .execute(&self.db)
        .await?;
        sqlx::query(
            r#"
            UPDATE limit_changes SET applied_at = now()
            WHERE account_id = $1 AND applied_at IS NULL AND effective_at <= now()
            "#,
        )
        .bind(from_account_id)
        .execute(&self.db)
        .await?;

        let (limit, balance): (Decimal, Decimal) =
            sqlx::query_as(r#"SELECT transfer_limit, balance FROM accounts WHERE id = $1"#)
                .bind(from_account_id)
                .fetch_one(&self.db)
                .await?;
        // Insufficient funds is checked BEFORE anything is posted: no OTP is
        // issued and the attempt never reaches the fraud rules. A rejected row
        // is still recorded - it feeds the user's history and the hijack
        // heuristic (3 overdraft attempts in 24h freezes the account).
        if amount > balance {
            sqlx::query(
                r#"
                INSERT INTO transfers (from_account_id, to_account_id, amount, status, note, status_reason)
                VALUES ($1, $2, $3, 'rejected', $4, $5)
                "#,
            )
            .bind(from_account_id)
            .bind(to_account_id)
            .bind(amount)
            .bind(note.as_deref())
            .bind(format!("insufficient funds: balance ${balance} is less than ${amount}"))
            .execute(&self.db)
            .await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.rejected",
                    json!({ "reason": "insufficient funds (pre-check)", "amount": amount.to_string() }),
                )
                .await?;
            self.check_overdraft_freeze(actor, from_account_id).await?;
            return Err(AppError::BadRequest(format!(
                "insufficient funds: balance ${balance} is less than ${amount}"
            )));
        }

        // ── Available-balance check ─────────────────────────────────────
        // Money already committed to this account's OUTSTANDING outgoing
        // transfers - those awaiting OTP ('pending') or held for staff review
        // ('on_hold') - is not spendable a second time. Comparing against the
        // raw balance would let a user promise the same dollars twice: e.g. a
        // $10k transfer parked on hold, then the balance drained by fresh
        // transfers, so the hold can never be released. We gate against the
        // AVAILABLE balance instead. (This is the early UX guard; confirm()
        // re-checks under the row lock for the race-safe guarantee.)
        let (reserved,): (Decimal,) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(amount), 0)
            FROM transfers
            WHERE from_account_id = $1 AND status IN ('pending', 'on_hold')
            "#,
        )
        .bind(from_account_id)
        .fetch_one(&self.db)
        .await?;
        let available = balance - reserved;
        if amount > available {
            return Err(AppError::BadRequest(format!(
                "amount exceeds your available balance of ${available} \
                 (${reserved} is reserved by pending or on-hold transfers)"
            )));
        }

        if amount > limit {
            return Err(AppError::BadRequest(format!(
                "amount exceeds this account's per-transfer limit of ${limit} - request a limit increase from the account page"
            )));
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
                "transfer created - OTP shown on screen (no Telegram linked)"
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
            SELECT id, from_account_id, to_account_id, amount, status, note, otp_hash, otp_attempts, created_at
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

        // OTP TTL: a pending transfer must be confirmed within 10 minutes.
        if Utc::now() - pending.created_at > chrono::Duration::minutes(10) {
            mark_rejected(&mut tx, transfer_id, "confirmation window expired").await?;
            tx.commit().await?;
            self.audit
                .record(Some(actor), "transfer.expired", json!({ "transfer_id": transfer_id }))
                .await?;
            return Err(AppError::Conflict(
                "the confirmation window has expired - please start the transfer again".into(),
            ));
        }

        // (2) The actor must own the source account. Without this check, any
        // logged-in user who learned a transfer id could confirm it - or kill
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
            let attempts = pending.otp_attempts + 1;
            if attempts >= 3 {
                // Three strikes: reject the transfer - a fraud signal in itself.
                mark_rejected(&mut tx, transfer_id, "too many invalid confirmation codes").await?;
                tx.commit().await?;
                self.audit
                    .record(
                        Some(actor),
                        "transfer.otp_lockout",
                        json!({ "transfer_id": transfer_id, "attempts": attempts }),
                    )
                    .await?;
                return Err(AppError::BadRequest(
                    "too many invalid codes - the transfer has been rejected".into(),
                ));
            }
            sqlx::query(r#"UPDATE transfers SET otp_attempts = $1 WHERE id = $2"#)
                .bind(attempts)
                .bind(transfer_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.otp_failed",
                    json!({ "transfer_id": transfer_id, "attempt": attempts }),
                )
                .await?;
            return Err(AppError::BadRequest(format!(
                "invalid confirmation code (attempt {attempts} of 3)"
            )));
        }

        // (4) Lock both account rows. ORDER BY id eliminates deadlock potential
        //     when two concurrent transfers touch the same pair of accounts in
        //     opposite directions.
        let (low, high) = if pending.from_account_id < pending.to_account_id {
            (pending.from_account_id, pending.to_account_id)
        } else {
            (pending.to_account_id, pending.from_account_id)
        };
        let accounts: Vec<(i64, AccountStatus, Decimal, Decimal)> = sqlx::query_as(
            r#"
            SELECT id, status, balance, transfer_limit
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

        // unwrap() is safe - we asserted len() == 2 just above.
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

            self.check_overdraft_freeze(actor, pending.from_account_id).await?;

            return Err(reject_with(&format!(
                "insufficient funds: balance ${} < ${}",
                from.2, pending.amount
            )));
        }

        // (5a) Reserved-funds re-check, under the same row lock as the balance.
        // Funds committed to OTHER on-hold transfers from this account are
        // earmarked pending staff review and must not be re-spent by this one.
        // Without this, a transfer parked on hold reserves nothing, so the
        // owner could drain the balance with fresh transfers and the held
        // transfer would later fail to release ("bugs out" on approval).
        // Excludes the current transfer; on-hold amounts are immutable and the
        // account row is locked FOR UPDATE, so this is race-safe.
        let (held_elsewhere,): (Decimal,) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(amount), 0)
            FROM transfers
            WHERE from_account_id = $1 AND status = 'on_hold' AND id <> $2
            "#,
        )
        .bind(pending.from_account_id)
        .bind(transfer_id)
        .fetch_one(&mut *tx)
        .await?;
        if from.2 - held_elsewhere < pending.amount {
            let reason = format!(
                "insufficient available funds: ${} of the ${} balance is reserved by transfers held for review",
                held_elsewhere, from.2
            );
            mark_rejected(&mut tx, transfer_id, &reason).await?;
            tx.commit().await?;
            self.audit
                .record(
                    Some(actor),
                    "transfer.rejected",
                    json!({
                        "transfer_id": transfer_id,
                        "reason": "funds reserved by on-hold transfers",
                        "balance": from.2.to_string(),
                        "reserved": held_elsewhere.to_string(),
                        "amount": pending.amount.to_string(),
                    }),
                )
                .await?;
            return Err(reject_with(&reason));
        }

        // (5b) Fraud rules - evaluated only on otherwise-payable transfers.
        // A match parks the transfer ON HOLD without moving money; the
        // customer submits a purpose + identity claim and staff decide.
        let hold_reason: Option<String> = if pending.amount >= Decimal::from(10_000) {
            Some("large transfer (>= $10,000)".into())
        } else if from.2 >= Decimal::from(5_000)
            && pending.amount < from.3
            && from.2 - pending.amount <= Decimal::from(49)
        {
            // Structuring: a sub-limit transfer that empties a sizeable
            // balance to within $49 - sized to dodge controls.
            Some("possible structuring: empties the account to within $49 while staying under the limit".into())
        } else if from.2 > Decimal::from(5_000)
            && pending.amount * Decimal::from(2) > from.2
        {
            Some("drains more than half of the account balance".into())
        } else {
            let (recent,): (i64,) = sqlx::query_as(
                r#"
                SELECT COUNT(*) FROM transfers
                WHERE from_account_id = $1 AND status IN ('completed', 'on_hold')
                  AND created_at > now() - interval '1 hour'
                "#,
            )
            .bind(pending.from_account_id)
            .fetch_one(&mut *tx)
            .await?;
            if recent >= 3 {
                Some("high velocity: 4+ transfers from this account within 1 hour".into())
            } else {
                None
            }
        };

        if let Some(reason) = hold_reason {
            sqlx::query(
                r#"UPDATE transfers SET status = 'on_hold', status_reason = $2, otp_hash = NULL WHERE id = $1"#,
            )
            .bind(transfer_id)
            .bind(&reason)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            self.audit
                .record(
                    Some(actor),
                    "transfer.held",
                    json!({ "transfer_id": transfer_id, "reason": reason }),
                )
                .await?;
            let msg = format!(
                "Your current transfer of ${} is ON HOLD ({reason}). It may be flagged as potentially illegitimate - submit the transfer purpose and your NRIC for staff review.",
                pending.amount
            );
            notify(&self.db, actor, &msg).await;
            self.otp_channel.send_note(actor, &msg).await;

            return Ok(Transfer {
                id: pending.id,
                from_account_id: pending.from_account_id,
                to_account_id: pending.to_account_id,
                amount: pending.amount,
                status: TransferStatus::OnHold,
                note: pending.note,
                status_reason: Some(reason),
                created_at: pending.created_at,
            });
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

        // Tell both parties who the other side was (toast + Telegram).
        let parties: Vec<(i64, i64, String)> = sqlx::query_as(
            r#"
            SELECT a.id, u.id, u.full_name
            FROM accounts a JOIN users u ON u.id = a.user_id
            WHERE a.id IN ($1, $2)
            "#,
        )
        .bind(pending.from_account_id)
        .bind(pending.to_account_id)
        .fetch_all(&self.db)
        .await
        .unwrap_or_default();
        let sender_name = parties
            .iter()
            .find(|p| p.0 == pending.from_account_id)
            .map(|p| p.2.clone())
            .unwrap_or_else(|| "another customer".into());
        let recipient = parties
            .iter()
            .find(|p| p.0 == pending.to_account_id)
            .cloned();

        let sender_msg = format!(
            "Your transfer of ${} to {} completed.",
            pending.amount,
            recipient.as_ref().map(|p| p.2.as_str()).unwrap_or("the recipient")
        );
        notify(&self.db, actor, &sender_msg).await;
        self.otp_channel.send_note(actor, &sender_msg).await;
        if let Some((_, recipient_id, _)) = recipient {
            let msg = format!("You received ${} from {}.", pending.amount, sender_name);
            notify(&self.db, recipient_id, &msg).await;
            self.otp_channel.send_note(recipient_id, &msg).await;
        }

        // (7) Audit outside the transaction - the audit_log row is its own
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

    async fn get_for_owner(&self, actor: i64, transfer_id: i64) -> Result<Transfer, AppError> {
        sqlx::query_as::<_, Transfer>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id, t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            JOIN accounts a ON a.id = t.from_account_id
            WHERE t.id = $1 AND a.user_id = $2
            "#,
        )
        .bind(transfer_id)
        .bind(actor)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("transfer {transfer_id} not found")))
    }

    async fn submit_review(
        &self,
        actor: i64,
        transfer_id: i64,
        purpose: &str,
        nric: &str,
    ) -> Result<(), AppError> {
        if purpose.trim().is_empty() || nric.trim().is_empty() {
            return Err(AppError::BadRequest(
                "both the purpose and your NRIC are required".into(),
            ));
        }
        // Only the owner of the source account may file, and only while held.
        let t = self.get_for_owner(actor, transfer_id).await?;
        if t.status != TransferStatus::OnHold {
            return Err(AppError::Conflict("this transfer is not on hold".into()));
        }

        sqlx::query(
            r#"
            INSERT INTO transfer_reviews (transfer_id, purpose, nric_claimed)
            VALUES ($1, $2, $3)
            ON CONFLICT (transfer_id) DO UPDATE
                SET purpose = EXCLUDED.purpose,
                    nric_claimed = EXCLUDED.nric_claimed,
                    submitted_at = now()
                WHERE transfer_reviews.decision IS NULL
            "#,
        )
        .bind(transfer_id)
        .bind(purpose.trim())
        .bind(nric.trim())
        .execute(&self.db)
        .await?;

        self.audit
            .record(
                Some(actor),
                "transfer.review_submitted",
                json!({ "transfer_id": transfer_id }),
            )
            .await?;
        Ok(())
    }

    async fn list_held(&self) -> Result<Vec<HeldRow>, AppError> {
        let rows = sqlx::query_as::<_, HeldRow>(
            r#"
            SELECT t.id, t.amount, t.status_reason, t.created_at,
                   fa.account_number AS from_number, ta.account_number AS to_number,
                   fu.id AS from_user_id, fu.full_name AS from_owner, fu.nric AS nric_on_file,
                   tu.full_name AS to_owner,
                   r.purpose, r.nric_claimed, r.submitted_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            LEFT JOIN transfer_reviews r ON r.transfer_id = t.id AND r.decision IS NULL
            WHERE t.status = 'on_hold'
            ORDER BY t.created_at ASC
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn release(&self, reviewer: i64, transfer_id: i64) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;

        let pending: PendingRow = sqlx::query_as(
            r#"
            SELECT id, from_account_id, to_account_id, amount, status, note, otp_hash, otp_attempts, created_at
            FROM transfers WHERE id = $1 FOR UPDATE
            "#,
        )
        .bind(transfer_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("transfer {transfer_id} not found")))?;
        if pending.status != TransferStatus::OnHold {
            return Err(AppError::Conflict("transfer is not on hold".into()));
        }

        // The customer must have filed a review request first.
        let review: Option<(i64,)> = sqlx::query_as(
            r#"SELECT id FROM transfer_reviews WHERE transfer_id = $1 AND decision IS NULL FOR UPDATE"#,
        )
        .bind(transfer_id)
        .fetch_optional(&mut *tx)
        .await?;
        if review.is_none() {
            return Err(AppError::Conflict(
                "the customer has not submitted a review request yet".into(),
            ));
        }

        // Same locked re-checks as confirm - the world may have changed.
        let (low, high) = if pending.from_account_id < pending.to_account_id {
            (pending.from_account_id, pending.to_account_id)
        } else {
            (pending.to_account_id, pending.from_account_id)
        };
        let accounts: Vec<(i64, AccountStatus, Decimal)> = sqlx::query_as(
            r#"SELECT id, status, balance FROM accounts WHERE id IN ($1, $2) ORDER BY id FOR UPDATE"#,
        )
        .bind(low)
        .bind(high)
        .fetch_all(&mut *tx)
        .await?;
        let from_ok = accounts
            .iter()
            .any(|a| a.0 == pending.from_account_id && a.1 == AccountStatus::Active && a.2 >= pending.amount);
        let to_ok = accounts
            .iter()
            .any(|a| a.0 == pending.to_account_id && a.1 == AccountStatus::Active);
        if !from_ok || !to_ok {
            mark_rejected(&mut tx, transfer_id, "release failed: account state or funds changed").await?;
            sqlx::query(
                r#"UPDATE transfer_reviews SET decided_by = $2, decided_at = now(), decision = 'denied' WHERE transfer_id = $1"#,
            )
            .bind(transfer_id)
            .bind(reviewer)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Err(AppError::Conflict(
                "cannot release: the account state or balance changed since the hold".into(),
            ));
        }

        apply_money_move(
            &mut tx,
            pending.from_account_id,
            pending.to_account_id,
            pending.amount,
            transfer_id,
        )
        .await?;
        sqlx::query(
            r#"UPDATE transfer_reviews SET decided_by = $2, decided_at = now(), decision = 'released' WHERE transfer_id = $1"#,
        )
        .bind(transfer_id)
        .bind(reviewer)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.audit
            .record(Some(reviewer), "transfer.released", json!({ "transfer_id": transfer_id }))
            .await?;
        // Tell both parties, with names - money moved on release.
        let parties: Vec<(i64, i64, String)> = sqlx::query_as(
            r#"
            SELECT a.id, u.id, u.full_name
            FROM accounts a JOIN users u ON u.id = a.user_id
            WHERE a.id IN ($1, $2)
            "#,
        )
        .bind(pending.from_account_id)
        .bind(pending.to_account_id)
        .fetch_all(&self.db)
        .await
        .unwrap_or_default();
        let sender = parties.iter().find(|p| p.0 == pending.from_account_id).cloned();
        let recipient = parties.iter().find(|p| p.0 == pending.to_account_id).cloned();
        if let Some((_, sender_id, _)) = sender.as_ref() {
            let msg = format!(
                "Good news - your held transfer of ${} to {} was reviewed and released.",
                pending.amount,
                recipient.as_ref().map(|p| p.2.as_str()).unwrap_or("the recipient")
            );
            notify(&self.db, *sender_id, &msg).await;
            self.otp_channel.send_note(*sender_id, &msg).await;
        }
        if let Some((_, recipient_id, _)) = recipient.as_ref() {
            let msg = format!(
                "You received ${} from {}.",
                pending.amount,
                sender.as_ref().map(|p| p.2.as_str()).unwrap_or("another customer")
            );
            notify(&self.db, *recipient_id, &msg).await;
            self.otp_channel.send_note(*recipient_id, &msg).await;
        }
        Ok(())
    }

    async fn deny(&self, reviewer: i64, transfer_id: i64, reason: &str) -> Result<(), AppError> {
        let updated = sqlx::query(
            r#"UPDATE transfers SET status = 'rejected', status_reason = $2 WHERE id = $1 AND status = 'on_hold'"#,
        )
        .bind(transfer_id)
        .bind(reason)
        .execute(&self.db)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(AppError::Conflict("transfer is not on hold".into()));
        }
        sqlx::query(
            r#"UPDATE transfer_reviews SET decided_by = $2, decided_at = now(), decision = 'denied' WHERE transfer_id = $1 AND decision IS NULL"#,
        )
        .bind(transfer_id)
        .bind(reviewer)
        .execute(&self.db)
        .await?;

        self.audit
            .record(
                Some(reviewer),
                "transfer.denied",
                json!({ "transfer_id": transfer_id, "reason": reason }),
            )
            .await?;
        if let Ok(Some((owner, amount))) = sqlx::query_as::<_, (i64, Decimal)>(
            r#"SELECT a.user_id, t.amount FROM accounts a JOIN transfers t ON t.from_account_id = a.id WHERE t.id = $1"#,
        )
        .bind(transfer_id)
        .fetch_optional(&self.db)
        .await
        {
            let msg = format!("Your held transfer of ${amount} was denied after review: {reason}");
            notify(&self.db, owner, &msg).await;
            self.otp_channel.send_note(owner, &msg).await;
        }
        Ok(())
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
