//! Generalized action-level OTP guard — Member 4's extended feature.
//!
//! The transfer flow has OTP confirmation built into its own table; this
//! service extends the same one-time-code protection to ANY sensitive action:
//! account opening, loan applications, and profile changes. The pattern:
//!
//!   1. `begin()` — store the action's inputs as a payload, generate a 6-digit
//!      code (argon2-hashed at rest), deliver it via the injected
//!      [`OtpChannel`] (Telegram, or on-screen fallback).
//!   2. The handler renders the shared `otp_confirm.html` page.
//!   3. `verify()` — single-use, owner-bound, 10-minute expiry. Returns the
//!      payload so the handler can finally perform the deferred action.

use std::sync::Arc;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::services::telegram_service::OtpChannel;

/// How long a code stays valid.
const OTP_TTL_MINUTES: i64 = 10;

pub struct ActionChallenge {
    pub action_id: i64,
    /// Plaintext code — only for on-screen display when not delivered.
    pub otp: String,
    /// `true` when the code went to the user's linked Telegram.
    pub delivered: bool,
}

#[async_trait]
pub trait ActionOtpService: Send + Sync {
    /// Step 1: record the pending action + send/show its code.
    async fn begin(
        &self,
        user_id: i64,
        purpose: &str,
        payload: serde_json::Value,
    ) -> Result<ActionChallenge, AppError>;

    /// Step 2: verify the code (owner-bound, single-use, expiring) and return
    /// the stored payload so the caller can perform the action.
    async fn verify(
        &self,
        user_id: i64,
        action_id: i64,
        purpose: &str,
        otp: &str,
    ) -> Result<serde_json::Value, AppError>;
}

pub struct PgActionOtpService {
    db: PgPool,
    otp_channel: Arc<dyn OtpChannel>,
}

impl PgActionOtpService {
    pub fn new(db: PgPool, otp_channel: Arc<dyn OtpChannel>) -> Self {
        Self { db, otp_channel }
    }
}

#[derive(sqlx::FromRow)]
struct ActionRow {
    otp_hash: String,
    payload: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[async_trait]
impl ActionOtpService for PgActionOtpService {
    async fn begin(
        &self,
        user_id: i64,
        purpose: &str,
        payload: serde_json::Value,
    ) -> Result<ActionChallenge, AppError> {
        let otp: String = {
            let mut rng = rand::thread_rng();
            format!("{:06}", rng.gen_range(0..1_000_000u32))
        };
        let salt = SaltString::generate(&mut OsRng);
        let otp_hash = Argon2::default()
            .hash_password(otp.as_bytes(), &salt)
            .map_err(AppError::from)?
            .to_string();

        let (action_id,): (i64,) = sqlx::query_as(
            r#"
            INSERT INTO action_otps (user_id, purpose, payload, otp_hash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(purpose)
        .bind(&payload)
        .bind(&otp_hash)
        .fetch_one(&self.db)
        .await?;

        let delivered = self.otp_channel.send_otp(user_id, &otp).await;
        if !delivered {
            tracing::info!(
                action_id,
                purpose,
                otp = %otp,
                "action OTP shown on screen (no Telegram linked)"
            );
        }

        Ok(ActionChallenge {
            action_id,
            otp,
            delivered,
        })
    }

    async fn verify(
        &self,
        user_id: i64,
        action_id: i64,
        purpose: &str,
        otp: &str,
    ) -> Result<serde_json::Value, AppError> {
        let mut tx = self.db.begin().await?;

        // Owner-bound + purpose-bound + unconsumed, locked against double use.
        let row: ActionRow = sqlx::query_as(
            r#"
            SELECT otp_hash, payload, created_at
            FROM action_otps
            WHERE id = $1 AND user_id = $2 AND purpose = $3 AND consumed_at IS NULL
            FOR UPDATE
            "#,
        )
        .bind(action_id)
        .bind(user_id)
        .bind(purpose)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::Conflict("this confirmation is no longer valid".into()))?;

        if Utc::now() - row.created_at > Duration::minutes(OTP_TTL_MINUTES) {
            return Err(AppError::Conflict(
                "the code has expired — please start the action again".into(),
            ));
        }

        let parsed = PasswordHash::new(&row.otp_hash)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("malformed OTP hash: {e}")))?;
        if Argon2::default()
            .verify_password(otp.trim().as_bytes(), &parsed)
            .is_err()
        {
            return Err(AppError::BadRequest("invalid confirmation code".into()));
        }

        sqlx::query(r#"UPDATE action_otps SET consumed_at = now() WHERE id = $1"#)
            .bind(action_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        Ok(row.payload)
    }
}
