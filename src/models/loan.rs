//! Loan model — owned by the Loans module (Member 5).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "loan_status", rename_all = "snake_case")]
pub enum LoanStatus {
    Pending,
    Approved,
    Active,
    PaidOff,
    Rejected,
}

impl LoanStatus {
    pub fn label(&self) -> &'static str {
        match self {
            LoanStatus::Pending => "Pending",
            LoanStatus::Approved => "Approved",
            LoanStatus::Active => "Active",
            LoanStatus::PaidOff => "Paid off",
            LoanStatus::Rejected => "Rejected",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct Loan {
    pub id: i64,
    pub user_id: i64,
    pub principal: Decimal,
    /// Annual rate as a decimal (e.g., 0.0525 = 5.25%).
    pub interest_rate: Decimal,
    pub term_months: i32,
    pub status: LoanStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Repayment {
    pub id: i64,
    pub loan_id: i64,
    pub amount: Decimal,
    pub paid_at: chrono::DateTime<chrono::Utc>,
}
