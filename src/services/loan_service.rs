//! Loan service — owned by the Loans module (Member 5).
//!
//! Uses a **simple-interest** model for clarity: total amount due is
//!     `principal * (1 + interest_rate * term_months / 12)`
//! and the outstanding balance is `total_due - sum_of_repayments`. A production
//! system would use amortization with monthly compounding; the simplified model
//! is documented in the report.

use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::account::AccountStatus;
use crate::models::loan::{Loan, LoanStatus, Repayment};
use crate::models::user::Role;

#[async_trait]
pub trait LoanService: Send + Sync {
    async fn apply(
        &self,
        user_id: i64,
        principal: Decimal,
        interest_rate: Decimal,
        term_months: i32,
    ) -> Result<Loan, AppError>;

    /// Record one staff approval (teller or admin) for a pending loan. The loan
    /// only flips to `approved` once it has BOTH a teller approval and an admin
    /// approval. Because each slot is keyed by role, the two approvals are
    /// guaranteed to come from two different people.
    async fn approve(
        &self,
        loan_id: i64,
        approver_id: i64,
        approver_role: Role,
    ) -> Result<Loan, AppError>;

    /// Staff-only. A single rejection is final.
    async fn reject(&self, loan_id: i64) -> Result<Loan, AppError>;

    /// `(teller_approved, admin_approved)` for the dual-approval UI.
    async fn approval_flags(&self, loan_id: i64) -> Result<(bool, bool), AppError>;

    async fn record_repayment(
        &self,
        loan_id: i64,
        amount: Decimal,
        account_id: i64,
    ) -> Result<Repayment, AppError>;
    async fn outstanding_balance(&self, loan_id: i64) -> Result<Decimal, AppError>;
    async fn get_by_id(&self, loan_id: i64) -> Result<Loan, AppError>;
    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Loan>, AppError>;
    async fn list_repayments(&self, loan_id: i64) -> Result<Vec<Repayment>, AppError>;

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

/// Total amount owed over the life of a loan (principal + simple interest).
///
/// `interest_rate` is the **annual** rate as a decimal (0.0525 = 5.25%).
fn total_due(principal: Decimal, interest_rate: Decimal, term_months: i32) -> Decimal {
    let months_factor = Decimal::from(term_months) / Decimal::from(12);
    principal + principal * interest_rate * months_factor
}

#[async_trait]
impl LoanService for PgLoanService {
    async fn apply(
        &self,
        user_id: i64,
        principal: Decimal,
        interest_rate: Decimal,
        term_months: i32,
    ) -> Result<Loan, AppError> {
        // Defence-in-depth — the table CHECK constraints catch these too,
        // but a clear AppError beats a SQL error in the UX.
        if principal <= Decimal::ZERO {
            return Err(AppError::BadRequest("principal must be positive".into()));
        }
        if interest_rate < Decimal::ZERO || interest_rate >= Decimal::ONE {
            return Err(AppError::BadRequest(
                "interest rate must be between 0 and 1 (e.g. 0.0525 for 5.25%)".into(),
            ));
        }
        if !(1..=360).contains(&term_months) {
            return Err(AppError::BadRequest(
                "term must be between 1 and 360 months".into(),
            ));
        }

        let loan = sqlx::query_as::<_, Loan>(
            r#"
            INSERT INTO loans (user_id, principal, interest_rate, term_months, status)
            VALUES ($1, $2, $3, $4, 'pending')
            RETURNING id, user_id, principal, interest_rate, term_months, status, created_at
            "#,
        )
        .bind(user_id)
        .bind(principal)
        .bind(interest_rate)
        .bind(term_months)
        .fetch_one(&self.db)
        .await?;

        tracing::info!(loan_id = loan.id, user_id, "loan application submitted");
        Ok(loan)
    }

    async fn approve(
        &self,
        loan_id: i64,
        approver_id: i64,
        approver_role: Role,
    ) -> Result<Loan, AppError> {
        // Only staff approvals count; the approval slot IS the approver's role.
        let slot = match approver_role {
            Role::Admin => Role::Admin,
            Role::Teller => Role::Teller,
            Role::Customer => return Err(AppError::Forbidden),
        };

        let mut tx = self.db.begin().await?;

        // Lock the loan — only a pending loan can collect approvals.
        let loan: Loan = sqlx::query_as::<_, Loan>(
            r#"
            SELECT id, user_id, principal, interest_rate, term_months, status, created_at
            FROM loans
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(loan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("loan {loan_id} not found")))?;

        if loan.status != LoanStatus::Pending {
            return Err(AppError::Conflict(format!(
                "loan {loan_id} is {} and cannot be approved",
                loan.status.label().to_lowercase()
            )));
        }

        // Record this role's approval. UNIQUE(loan_id, role) means a second
        // approval from the same role is rejected as a no-op.
        let inserted = sqlx::query(
            r#"
            INSERT INTO loan_approvals (loan_id, approver_user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (loan_id, role) DO NOTHING
            "#,
        )
        .bind(loan_id)
        .bind(approver_id)
        .bind(slot)
        .execute(&mut *tx)
        .await?;

        if inserted.rows_affected() == 0 {
            return Err(AppError::Conflict(
                "this loan already has an approval from a staff member of your role".into(),
            ));
        }

        // Do we now hold BOTH a teller approval and an admin approval?
        let counts: (i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE role = 'teller'),
                COUNT(*) FILTER (WHERE role = 'admin')
            FROM loan_approvals
            WHERE loan_id = $1
            "#,
        )
        .bind(loan_id)
        .fetch_one(&mut *tx)
        .await?;

        let fully_approved = counts.0 > 0 && counts.1 > 0;

        let result = if fully_approved {
            sqlx::query_as::<_, Loan>(
                r#"
                UPDATE loans
                SET status = 'approved', decided_at = now()
                WHERE id = $1
                RETURNING id, user_id, principal, interest_rate, term_months, status, created_at
                "#,
            )
            .bind(loan_id)
            .fetch_one(&mut *tx)
            .await?
        } else {
            loan
        };

        tx.commit().await?;
        tracing::info!(loan_id, ?slot, fully_approved, "loan approval recorded");
        Ok(result)
    }

    async fn approval_flags(&self, loan_id: i64) -> Result<(bool, bool), AppError> {
        let counts: (i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) FILTER (WHERE role = 'teller'),
                COUNT(*) FILTER (WHERE role = 'admin')
            FROM loan_approvals
            WHERE loan_id = $1
            "#,
        )
        .bind(loan_id)
        .fetch_one(&self.db)
        .await?;
        Ok((counts.0 > 0, counts.1 > 0))
    }

    async fn reject(&self, loan_id: i64) -> Result<Loan, AppError> {
        let loan = sqlx::query_as::<_, Loan>(
            r#"
            UPDATE loans
            SET status = 'rejected', decided_at = now()
            WHERE id = $1 AND status = 'pending'
            RETURNING id, user_id, principal, interest_rate, term_months, status, created_at
            "#,
        )
        .bind(loan_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| {
            AppError::Conflict(format!("loan {loan_id} is not pending or does not exist"))
        })?;

        tracing::info!(loan_id, "loan rejected");
        Ok(loan)
    }

    async fn record_repayment(
        &self,
        loan_id: i64,
        amount: Decimal,
        account_id: i64,
    ) -> Result<Repayment, AppError> {
        if amount <= Decimal::ZERO {
            return Err(AppError::BadRequest("repayment must be positive".into()));
        }

        let mut tx = self.db.begin().await?;

        // Lock the loan row so concurrent repayments can't both push the
        // status from 'active' to 'paid_off' independently.
        let loan: Loan = sqlx::query_as::<_, Loan>(
            r#"
            SELECT id, user_id, principal, interest_rate, term_months, status, created_at
            FROM loans
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(loan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("loan {loan_id} not found")))?;

        if loan.status != LoanStatus::Approved && loan.status != LoanStatus::Active {
            return Err(AppError::Conflict(format!(
                "cannot repay a loan that is {}",
                loan.status.label().to_lowercase()
            )));
        }

        // Lock the funding account and make sure it can actually cover the
        // payment. This is what makes a repayment move real money instead of
        // just writing a ledger row.
        let funding: (AccountStatus, Decimal) = sqlx::query_as(
            r#"SELECT status, balance FROM accounts WHERE id = $1 FOR UPDATE"#,
        )
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("account {account_id} not found")))?;

        if funding.0 != AccountStatus::Active {
            return Err(AppError::Conflict(format!(
                "funding account is {} and cannot be used",
                funding.0.label().to_lowercase()
            )));
        }
        if funding.1 < amount {
            return Err(AppError::Conflict(format!(
                "insufficient funds: account balance ${} is less than ${}",
                funding.1, amount
            )));
        }

        // Debit the funding account.
        sqlx::query(r#"UPDATE accounts SET balance = balance - $1 WHERE id = $2"#)
            .bind(amount)
            .bind(account_id)
            .execute(&mut *tx)
            .await?;

        let repayment: Repayment = sqlx::query_as::<_, Repayment>(
            r#"
            INSERT INTO repayments (loan_id, amount, account_id)
            VALUES ($1, $2, $3)
            RETURNING id, loan_id, amount, account_id, paid_at
            "#,
        )
        .bind(loan_id)
        .bind(amount)
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;

        // Recompute outstanding inside the same txn.
        let paid: (Decimal,) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(amount), 0)::NUMERIC FROM repayments WHERE loan_id = $1"#,
        )
        .bind(loan_id)
        .fetch_one(&mut *tx)
        .await?;
        let due = total_due(loan.principal, loan.interest_rate, loan.term_months);
        let outstanding = due - paid.0;

        let new_status = if outstanding <= Decimal::ZERO {
            LoanStatus::PaidOff
        } else {
            LoanStatus::Active
        };
        sqlx::query(r#"UPDATE loans SET status = $1 WHERE id = $2"#)
            .bind(new_status)
            .bind(loan_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            loan_id,
            repayment_id = repayment.id,
            outstanding = %outstanding,
            new_status = ?new_status,
            "repayment recorded"
        );
        Ok(repayment)
    }

    async fn outstanding_balance(&self, loan_id: i64) -> Result<Decimal, AppError> {
        let loan = self.get_by_id(loan_id).await?;
        let paid: (Decimal,) = sqlx::query_as(
            r#"SELECT COALESCE(SUM(amount), 0)::NUMERIC FROM repayments WHERE loan_id = $1"#,
        )
        .bind(loan_id)
        .fetch_one(&self.db)
        .await?;
        let due = total_due(loan.principal, loan.interest_rate, loan.term_months);
        Ok((due - paid.0).max(Decimal::ZERO).round_dp(2))
    }

    async fn get_by_id(&self, loan_id: i64) -> Result<Loan, AppError> {
        sqlx::query_as::<_, Loan>(
            r#"
            SELECT id, user_id, principal, interest_rate, term_months, status, created_at
            FROM loans
            WHERE id = $1
            "#,
        )
        .bind(loan_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("loan {loan_id} not found")))
    }

    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Loan>, AppError> {
        let rows = sqlx::query_as::<_, Loan>(
            r#"
            SELECT id, user_id, principal, interest_rate, term_months, status, created_at
            FROM loans
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn list_repayments(&self, loan_id: i64) -> Result<Vec<Repayment>, AppError> {
        let rows = sqlx::query_as::<_, Repayment>(
            r#"
            SELECT id, loan_id, amount, account_id, paid_at
            FROM repayments
            WHERE loan_id = $1
            ORDER BY paid_at DESC
            "#,
        )
        .bind(loan_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    // ── Admin Dashboard hooks ────────────────────────────────────────

    async fn pending_applications(&self) -> Result<Vec<Loan>, AppError> {
        let rows = sqlx::query_as::<_, Loan>(
            r#"
            SELECT id, user_id, principal, interest_rate, term_months, status, created_at
            FROM loans
            WHERE status = 'pending'
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn portfolio_outstanding(&self) -> Result<Decimal, AppError> {
        // Sum the outstanding balance across every approved/active loan.
        // Cheaper than N+1 round trips per loan.
        let rows: Vec<(Decimal, Decimal, i32, Decimal)> = sqlx::query_as(
            r#"
            SELECT
                l.principal,
                l.interest_rate,
                l.term_months,
                COALESCE((SELECT SUM(amount) FROM repayments WHERE loan_id = l.id), 0)::NUMERIC AS paid
            FROM loans l
            WHERE l.status IN ('approved', 'active')
            "#,
        )
        .fetch_all(&self.db)
        .await?;

        let total: Decimal = rows
            .into_iter()
            .map(|(principal, rate, term, paid)| {
                (total_due(principal, rate, term) - paid).max(Decimal::ZERO)
            })
            .sum();
        Ok(total.round_dp(2))
    }
}
