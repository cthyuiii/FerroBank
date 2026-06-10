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
    /// Denormalized display name ("First [Middle] Last").
    pub full_name: String,
    /// Structured name parts. Nullable for rows created before migration 008.
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub role: Role,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    /// The name to greet the user with: first name, falling back to the first
    /// word of `full_name` for legacy rows.
    pub fn given_name(&self) -> String {
        self.first_name.clone().unwrap_or_else(|| {
            self.full_name
                .split_whitespace()
                .next()
                .unwrap_or("there")
                .to_string()
        })
    }
}

/// Input shape for `AuthService::register`.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub password: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    pub last_name: String,
    pub role: Role,
}

impl NewUser {
    /// Display name assembled from the structured parts.
    pub fn full_name(&self) -> String {
        match self.middle_name.as_deref().map(str::trim) {
            Some(m) if !m.is_empty() => {
                format!("{} {} {}", self.first_name.trim(), m, self.last_name.trim())
            }
            _ => format!("{} {}", self.first_name.trim(), self.last_name.trim()),
        }
    }
}
