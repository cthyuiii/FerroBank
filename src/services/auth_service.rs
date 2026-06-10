//! Auth service — owned by the Auth module (Member 2).
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
        // Same error for "user not found" and "wrong password" — never leak which one.
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

