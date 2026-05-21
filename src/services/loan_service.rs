//! Loan service — owned by the Loans module (Member 5).

use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::loan::{Loan, Repayment};

#[async_trait]
pub trait LoanService: Send + Sync {
    async fn apply(
        &self,
        user_id: i64,
        principal: Decimal,
        interest_rate: Decimal,
        term_months: i32,
    ) -> Result<Loan, AppError>;

    /// Admin-only.
    async fn approve(&self, loan_id: i64) -> Result<Loan, AppError>;

    /// Admin-only.
    async fn reject(&self, loan_id: i64) -> Result<Loan, AppError>;

    async fn record_repayment(&self, loan_id: i64, amount: Decimal) -> Result<Repayment, AppError>;
    async fn outstanding_balance(&self, loan_id: i64) -> Result<Decimal, AppError>;
    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Loan>, AppError>;

    // ── Admin Dashboard hooks ────────────────────────────────────────
    async fn pending_applications(&self) -> Result<Vec<Loan>, AppError>;
    async fn portfolio_outstanding(&self) -> Result<Decimal, AppError>;
}

pub struct PgLoanService {
    pub db: PgPool,
}

impl PgLoanService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl LoanService for PgLoanService {
    async fn apply(
        &self,
        _user_id: i64,
        _principal: Decimal,
        _interest_rate: Decimal,
        _term_months: i32,
    ) -> Result<Loan, AppError> {
        // TODO(Member 5):
        //   - Validate: principal > 0, interest_rate in [0, 1), term_months in 1..=360.
        //   - INSERT loan with status='pending'.
        todo!("LoanService::apply")
    }

    async fn approve(&self, _loan_id: i64) -> Result<Loan, AppError> {
        // TODO(Member 5): UPDATE loans SET status='approved' WHERE id=$1 AND status='pending'.
        todo!("LoanService::approve")
    }

    async fn reject(&self, _loan_id: i64) -> Result<Loan, AppError> {
        todo!("LoanService::reject")
    }

    async fn record_repayment(
        &self,
        _loan_id: i64,
        _amount: Decimal,
    ) -> Result<Repayment, AppError> {
        // TODO(Member 5):
        //   - INSERT repayment row.
        //   - If outstanding balance hits zero, UPDATE loans SET status='paid_off'.
        todo!("LoanService::record_repayment")
    }

    async fn outstanding_balance(&self, _loan_id: i64) -> Result<Decimal, AppError> {
        // TODO(Member 5):
        //   - SELECT principal + total_interest - SUM(repayments.amount).
        //   - Use the standard amortization formula for total_interest.
        todo!("LoanService::outstanding_balance")
    }

    async fn list_for_user(&self, _user_id: i64) -> Result<Vec<Loan>, AppError> {
        todo!("LoanService::list_for_user")
    }

    // ── Admin Dashboard hooks ────────────────────────────────────────

    async fn pending_applications(&self) -> Result<Vec<Loan>, AppError> {
        Ok(vec![])
    }

    async fn portfolio_outstanding(&self) -> Result<Decimal, AppError> {
        Ok(Decimal::ZERO)
    }
}
