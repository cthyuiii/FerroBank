//! Auth service — owned by the Auth module (Member 2).
//!
//! Responsibilities:
//!   - hash & verify passwords with argon2
//!   - look up users for the session
//!   - create new user records on registration

use async_trait::async_trait;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::user::{NewUser, User};

#[async_trait]
pub trait AuthService: Send + Sync {
    async fn register(&self, new_user: NewUser) -> Result<User, AppError>;
    async fn login(&self, email: &str, password: &str) -> Result<User, AppError>;
    async fn find_by_id(&self, id: i64) -> Result<User, AppError>;
}

pub struct PgAuthService {
    pub db: PgPool,
}

impl PgAuthService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AuthService for PgAuthService {
    async fn register(&self, _new_user: NewUser) -> Result<User, AppError> {
        // TODO(Member 2):
        //   1. Hash password with argon2 (PasswordHasher::hash_password).
        //   2. INSERT INTO users (email, password_hash, full_name, role)
        //      VALUES (...) RETURNING *.
        //   3. Map unique-violation on email to AppError::Conflict("email already registered").
        todo!("AuthService::register")
    }

    async fn login(&self, _email: &str, _password: &str) -> Result<User, AppError> {
        // TODO(Member 2):
        //   1. SELECT * FROM users WHERE email = $1.
        //   2. Use argon2 PasswordVerifier to compare against stored hash.
        //   3. Return AppError::Unauthorized for BOTH missing user and wrong password
        //      (don't leak which one failed).
        todo!("AuthService::login")
    }

    async fn find_by_id(&self, _id: i64) -> Result<User, AppError> {
        // TODO(Member 2): SELECT * FROM users WHERE id = $1.
        todo!("AuthService::find_by_id")
    }
}
