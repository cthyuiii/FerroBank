//! Account model — owned by the Accounts module (Member 3).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "account_type", rename_all = "snake_case")]
pub enum AccountType {
    Savings,
    Checking,
}

impl AccountType {
    pub fn label(&self) -> &'static str {
        match self {
            AccountType::Savings => "Savings",
            AccountType::Checking => "Checking",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "account_status", rename_all = "snake_case")]
pub enum AccountStatus {
    Pending,
    Active,
    Frozen,
    Closed,
}

impl AccountStatus {
    pub fn label(&self) -> &'static str {
        match self {
            AccountStatus::Pending => "Pending approval",
            AccountStatus::Active => "Active",
            AccountStatus::Frozen => "Frozen",
            AccountStatus::Closed => "Closed",
        }
    }

    /// Tailwind classes for a status pill (used by the admin tables).
    pub fn badge(&self) -> &'static str {
        match self {
            AccountStatus::Pending => "bg-blue-50 text-blue-700 border-blue-200",
            AccountStatus::Active => "bg-emerald-50 text-emerald-700 border-emerald-200",
            AccountStatus::Frozen => "bg-amber-50 text-amber-700 border-amber-200",
            AccountStatus::Closed => "bg-stone-100 text-stone-600 border-stone-200",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct Account {
    pub id: i64,
    pub user_id: i64,
    pub account_number: String,
    pub kind: AccountType,
    pub status: AccountStatus,
    pub balance: Decimal,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
