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
    /// Account credited with the principal once the loan is fully approved.
    pub disbursement_account_id: Option<i64>,
    /// When the next monthly repayment is due (set on approval, advanced on
    /// every repayment). `None` until approved or once paid off.
    pub next_payment_due: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Loan {
    /// Annual interest rate rendered as a percentage string, e.g. `0.0525` → `"5.25%"`.
    /// Templates call this so the UI never shows a raw 4-decimal fraction.
    pub fn rate_pct(&self) -> String {
        let pct = (self.interest_rate * Decimal::from(100)).round_dp(2).normalize();
        format!("{pct}%")
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct Repayment {
    pub id: i64,
    pub loan_id: i64,
    pub amount: Decimal,
    /// Account the repayment was drawn from. `None` for legacy/seed rows.
    pub account_id: Option<i64>,
    pub paid_at: chrono::DateTime<chrono::Utc>,
}
