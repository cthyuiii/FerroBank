//! User model — owned by the Auth module (Member 2).
//!
//! The `Role` enum is also referenced by the Platform Lead's auth middleware,
//! so its variants are part of the cross-module contract — don't rename them
//! without coordinating in the team chat.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Maps to the Postgres `user_role` enum (see `migrations/001_init.sql`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "snake_case")]
pub enum Role {
    Customer,
    Teller,
    Admin,
}

impl Role {
    pub fn label(&self) -> &'static str {
        match self {
            Role::Customer => "Customer",
            Role::Teller => "Teller",
            Role::Admin => "Admin",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub password_hash: String,
    pub full_name: String,
    pub role: Role,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Input shape for `AuthService::register`.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub password: String,
    pub full_name: String,
    pub role: Role,
}
