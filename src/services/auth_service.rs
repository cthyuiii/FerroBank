//! Auth service - owned by the Auth module (Member 2).
//!
//! Responsibilities:
//!   - hash & verify passwords with argon2id
//!   - look up users for the session
//!   - create new user records on registration

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_trait::async_trait;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::user::{NewUser, User};

#[async_trait]
pub trait AuthService: Send + Sync {
    async fn register(&self, new_user: NewUser) -> Result<User, AppError>;
    async fn login(&self, email: &str, password: &str) -> Result<User, AppError>;
    async fn find_by_id(&self, id: i64) -> Result<User, AppError>;
    /// Profile change (OTP-gated by the caller): set a new login email.
    async fn change_email(&self, user_id: i64, new_email: &str) -> Result<(), AppError>;
    /// Profile change (OTP-gated by the caller): set a new password.
    async fn change_password(&self, user_id: i64, new_password: &str) -> Result<(), AppError>;
}

pub struct PgAuthService {
    // Private: callers go through the trait methods, never the pool directly.
    db: PgPool,
}

impl PgAuthService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Hash a plaintext password with argon2id and the default OWASP parameters.
    fn hash_password(plaintext: &str) -> Result<String, AppError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(plaintext.as_bytes(), &salt)?;
        Ok(hash.to_string())
    }

    /// Verify a plaintext password against a stored argon2 hash.
    /// Returns `Ok(())` on match, `Err(AppError::Unauthorized)` on mismatch.
    fn verify_password(plaintext: &str, stored_hash: &str) -> Result<(), AppError> {
        let parsed = PasswordHash::new(stored_hash)?;
        Argon2::default()
            .verify_password(plaintext.as_bytes(), &parsed)
            .map_err(|_| AppError::Unauthorized)
    }
}

#[async_trait]
impl AuthService for PgAuthService {
    async fn register(&self, new_user: NewUser) -> Result<User, AppError> {
        let password_hash = Self::hash_password(&new_user.password)?;

        let full_name = new_user.full_name();
        let user = sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (email, password_hash, full_name, first_name, middle_name, last_name, nric, role)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, email, password_hash, full_name, first_name, middle_name, last_name, role, created_at
            "#,
        )
        .bind(&new_user.email)
        .bind(&password_hash)
        .bind(&full_name)
        .bind(new_user.first_name.trim())
        .bind(new_user.middle_name.as_deref().map(str::trim).filter(|m| !m.is_empty()))
        .bind(new_user.last_name.trim())
        .bind(new_user.nric.as_deref().map(str::trim).filter(|n| !n.is_empty()))
        .bind(new_user.role)
        .fetch_one(&self.db)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                AppError::Conflict("email already registered".into())
            }
            _ => AppError::from(e),
        })?;

        tracing::info!(user_id = user.id, email = %user.email, "registered new user");
        Ok(user)
    }

    async fn login(&self, email: &str, password: &str) -> Result<User, AppError> {
        // Same error for "user not found" and "wrong password" - never leak which one.
        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, email, password_hash, full_name, first_name, middle_name, last_name, role, created_at
            FROM users
            WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(&self.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

        Self::verify_password(password, &user.password_hash)?;

        // Reset the inactivity clock: without this, a session that expired
        // earlier would leave a stale last_activity_at and instantly expire
        // the NEXT login too (the /login?expired=1 loop).
        sqlx::query(r#"UPDATE users SET last_activity_at = now() WHERE id = $1"#)
            .bind(user.id)
            .execute(&self.db)
            .await?;

        tracing::info!(user_id = user.id, email = %user.email, "user logged in");
        Ok(user)
    }

    async fn find_by_id(&self, id: i64) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, email, password_hash, full_name, first_name, middle_name, last_name, role, created_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {id} not found")))
    }

    async fn change_email(&self, user_id: i64, new_email: &str) -> Result<(), AppError> {
        sqlx::query(r#"UPDATE users SET email = $1 WHERE id = $2"#)
            .bind(new_email.trim())
            .bind(user_id)
            .execute(&self.db)
            .await
            .map_err(|e| match &e {
                sqlx::Error::Database(db) if db.is_unique_violation() => {
                    AppError::Conflict("that email is already registered".into())
                }
                _ => AppError::from(e),
            })?;
        tracing::info!(user_id, "email changed");
        Ok(())
    }

    async fn change_password(&self, user_id: i64, new_password: &str) -> Result<(), AppError> {
        let hash = Self::hash_password(new_password)?;
        sqlx::query(r#"UPDATE users SET password_hash = $1 WHERE id = $2"#)
            .bind(&hash)
            .bind(user_id)
            .execute(&self.db)
            .await?;
        tracing::info!(user_id, "password changed");
        Ok(())
    }
}


// ── Login device tracking ────────────────────────────────────────────

/// Classify a User-Agent into a coarse browser family.
fn browser_family(user_agent: &str) -> &'static str {
    let ua = user_agent;
    if ua.contains("Edg/") || ua.contains("Edge/") {
        "Edge"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("Chromium/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Other"
    }
}

/// Is this user blocked from signing in from this browser + network?
/// Set after 3 failed step-up codes; expires automatically.
pub async fn origin_blocked(
    db: &sqlx::PgPool,
    user_id: i64,
    user_agent: &str,
    ip: &str,
) -> bool {
    let browser = browser_family(user_agent);
    sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM blocked_origins
            WHERE user_id = $1 AND browser = $2 AND ip = $3 AND blocked_until > now()
        )
        "#,
    )
    .bind(user_id)
    .bind(browser)
    .bind(ip)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

/// Block an origin for `hours` after a failed step-up, with audit + alerts.
pub async fn block_origin(
    db: &sqlx::PgPool,
    otp_channel: &std::sync::Arc<dyn crate::services::telegram_service::OtpChannel>,
    user_id: i64,
    user_agent: &str,
    ip: &str,
    hours: i32,
) {
    let browser = browser_family(user_agent);
    let _ = sqlx::query(
        r#"
        INSERT INTO blocked_origins (user_id, browser, ip, reason, blocked_until)
        VALUES ($1, $2, $3, 'three failed step-up codes', now() + ($4 || ' hours')::interval)
        "#,
    )
    .bind(user_id)
    .bind(browser)
    .bind(ip)
    .bind(hours.to_string())
    .execute(db)
    .await;
    let _ = sqlx::query(
        r#"INSERT INTO audit_log (actor_user_id, event, payload) VALUES ($1, 'auth.login.origin_blocked', $2)"#,
    )
    .bind(user_id)
    .bind(serde_json::json!({ "browser": browser, "ip": ip, "hours": hours }))
    .execute(db)
    .await;
    let msg = format!(
        "Security alert - sign-in verification failed 3 times from a new origin ({browser}, {ip}). That device/network is blocked for {hours} hours. If this wasn't you, change your password now."
    );
    crate::services::audit_service::notify(db, user_id, &msg).await;
    otp_channel.send_note(user_id, &msg).await;
    tracing::warn!(user_id, browser, ip, "origin blocked after failed step-up");
}

/// Is this sign-in coming from a first-seen browser or network for this
/// user? Used by the risk-based step-up: known origins log in with just the
/// password, first-seen origins must also present a one-time code. A user
/// with no history at all (their very first login) is NOT "new".
pub async fn login_origin_is_new(
    db: &sqlx::PgPool,
    user_id: i64,
    user_agent: &str,
    ip: &str,
) -> bool {
    let browser = browser_family(user_agent);
    let history: Option<(i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT COUNT(*)::BIGINT,
               COUNT(*) FILTER (WHERE browser = $2)::BIGINT,
               COUNT(*) FILTER (WHERE ip = $3)::BIGINT
        FROM login_sessions WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .bind(browser)
    .bind(ip)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let (total, same_browser, same_ip) = history.unwrap_or((0, 0, 0));
    total > 0 && (same_browser == 0 || same_ip == 0)
}

/// Record a successful sign-in with its browser and source IP, flagging
/// first-seen devices and networks. New-device/new-network logins notify the
/// user (toast + Telegram) and land in the audit log so staff can fold them
/// into fraud review.
pub async fn record_login_session(
    db: &sqlx::PgPool,
    otp_channel: &std::sync::Arc<dyn crate::services::telegram_service::OtpChannel>,
    user_id: i64,
    user_agent: &str,
    ip: &str,
) {
    let browser = browser_family(user_agent);

    // First-seen checks against this user's history (any prior session at all
    // is required - the very first login of a fresh account isn't "new").
    let history: Option<(i64, i64, i64)> = sqlx::query_as(
        r#"
        SELECT COUNT(*)::BIGINT,
               COUNT(*) FILTER (WHERE browser = $2)::BIGINT,
               COUNT(*) FILTER (WHERE ip = $3)::BIGINT
        FROM login_sessions WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .bind(browser)
    .bind(ip)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let (total, same_browser, same_ip) = history.unwrap_or((0, 0, 0));
    let is_new_device = total > 0 && same_browser == 0;
    let is_new_network = total > 0 && same_ip == 0;

    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO login_sessions (user_id, browser, user_agent, ip, is_new_device, is_new_network)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user_id)
    .bind(browser)
    .bind(user_agent)
    .bind(ip)
    .bind(is_new_device)
    .bind(is_new_network)
    .execute(db)
    .await
    {
        tracing::warn!(user_id, error = %e, "failed to record login session");
        return;
    }

    if is_new_device || is_new_network {
        let what = match (is_new_device, is_new_network) {
            (true, true) => "a new device AND a new network",
            (true, false) => "a new device",
            _ => "a new network",
        };
        let msg = format!(
            "Security alert - your account just signed in from {what} ({browser}, {ip}). If this wasn't you, change your password immediately."
        );
        crate::services::audit_service::notify(db, user_id, &msg).await;
        otp_channel.send_note(user_id, &msg).await;
        let _ = sqlx::query(
            r#"INSERT INTO audit_log (actor_user_id, event, payload) VALUES ($1, 'auth.login.new_origin', $2)"#,
        )
        .bind(user_id)
        .bind(serde_json::json!({ "browser": browser, "ip": ip, "new_device": is_new_device, "new_network": is_new_network }))
        .execute(db)
        .await;
    }

    tracing::info!(user_id, browser, ip, is_new_device, is_new_network, "login session recorded");
}
