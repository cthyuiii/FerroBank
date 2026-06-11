//! Admin service - owned by the Platform Lead (Member 1).
//!
//! Composes read-only data from every other module into the dashboard view-model.
//! Holds Arcs of the other service traits so it's testable with mocks.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool};

use crate::errors::AppError;
use crate::models::account::{AccountStatus, AccountType};
use crate::models::transfer::TransferStatus;
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::audit_service::{AuditEntry, AuditService};
use crate::services::loan_service::LoanService;

/// Threshold (dollars) at or above which a transfer is treated as noteworthy.
const LARGE_TRANSFER_THRESHOLD: i64 = 10_000;

/// One row of the admin "all accounts" table - account joined to its owner.
#[derive(Debug, Clone, FromRow)]
pub struct AdminAccountRow {
    pub id: i64,
    pub user_id: i64,
    pub account_number: String,
    pub kind: AccountType,
    pub status: AccountStatus,
    pub balance: Decimal,
    pub created_at: DateTime<Utc>,
    pub owner_name: String,
    pub owner_email: String,
}

/// One row of the admin "all transfers" table - transfer joined to both owners.
#[derive(Debug, Clone, FromRow)]
pub struct AdminTransferRow {
    pub id: i64,
    pub from_account_id: i64,
    pub to_account_id: i64,
    pub from_user_id: i64,
    pub to_user_id: i64,
    pub from_number: String,
    pub to_number: String,
    pub from_owner: String,
    pub to_owner: String,
    pub amount: Decimal,
    pub status: TransferStatus,
    pub note: Option<String>,
    pub status_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl AdminTransferRow {
    /// Why this transfer is noteworthy: an explicit rejection reason, or a
    /// large-amount flag. `None` for ordinary transfers.
    pub fn flag_reason(&self) -> Option<String> {
        if let Some(reason) = &self.status_reason {
            return Some(reason.clone());
        }
        if self.amount >= Decimal::from(LARGE_TRANSFER_THRESHOLD) {
            return Some("Large transfer (>= $10,000)".to_string());
        }
        None
    }
}

/// One row of the admin users picker - used to open an account on someone's behalf.
#[derive(Debug, Clone, FromRow)]
pub struct AdminUserRow {
    pub id: i64,
    pub email: String,
    pub full_name: String,
    pub role: Role,
}

/// A transfer surfaced by the fraud rules, with a human-readable reason computed
/// in SQL (so aggregate rules like velocity can be explained, not just per-row ones).
#[derive(Debug, Clone, FromRow)]
pub struct FlaggedTransfer {
    pub id: i64,
    pub from_user_id: i64,
    pub to_user_id: i64,
    pub from_owner: String,
    pub to_owner: String,
    pub amount: Decimal,
    pub status: TransferStatus,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

/// One pending-loan row for the dashboard, joined to the applicant's name so the
/// dashboard never shows a raw user id.
#[derive(Debug, Clone, FromRow)]
pub struct AdminLoanRow {
    pub id: i64,
    pub user_id: i64,
    pub applicant_name: String,
    pub principal: Decimal,
    pub interest_rate: Decimal,
    pub term_months: i32,
}

impl AdminLoanRow {
    /// Annual rate as a percentage string, e.g. "5.25%".
    pub fn rate_pct(&self) -> String {
        let pct = (self.interest_rate * Decimal::from(100)).round_dp(2).normalize();
        format!("{pct}%")
    }
}

/// Pre-computed snapshot shown on `/admin/dashboard`. Transfers and loans are
/// pre-joined to owner/applicant names so the view shows people, not ids.
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub active_accounts: i64,
    pub total_deposits: Decimal,
    pub portfolio_outstanding: Decimal,
    pub pending_loans: usize,
    pub recent_transfers: Vec<AdminTransferRow>,
    pub flagged_transfers: Vec<FlaggedTransfer>,
    pub pending_loans_list: Vec<AdminLoanRow>,
    pub recent_audit: Vec<AuditEntry>,
}

#[async_trait]
pub trait AdminService: Send + Sync {
    async fn snapshot(&self) -> Result<DashboardSnapshot, AppError>;

    /// Every account in the bank, joined to its owner, newest first.
    async fn all_accounts(&self) -> Result<Vec<AdminAccountRow>, AppError>;
    /// Every transfer, joined to both parties, newest first. Optional inclusive
    /// date bounds (`from`/`to`) filter by `created_at`.
    async fn all_transfers(
        &self,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> Result<Vec<AdminTransferRow>, AppError>;
    /// Customers only - accounts may only be opened for customers, so the
    /// "open an account" picker must not offer staff users.
    async fn customers(&self) -> Result<Vec<AdminUserRow>, AppError>;

    /// Every transfer touching any of one user's accounts, for the per-user
    /// activity view.
    async fn transfers_for_user(&self, user_id: i64) -> Result<Vec<AdminTransferRow>, AppError>;
}

pub struct PgAdminService {
    // Private state - consumers use the `AdminService` trait. The dependent
    // services are injected after construction via `with_services` (a builder),
    // because `PgAdminService` is built in `main.rs` before they exist.
    db: PgPool,
    accounts: Option<Arc<dyn AccountService>>,
    loans: Option<Arc<dyn LoanService>>,
    audit: Option<Arc<dyn AuditService>>,
    // Transfer reads are done with direct joined SQL here (so the dashboard can
    // show owner names), so the TransferService isn't injected.
}

impl PgAdminService {
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            accounts: None,
            loans: None,
            audit: None,
        }
    }

    /// Inject the dependent services. Called by main.rs after all services are built.
    pub fn with_services(
        mut self,
        accounts: Arc<dyn AccountService>,
        loans: Arc<dyn LoanService>,
        audit: Arc<dyn AuditService>,
    ) -> Self {
        self.accounts = Some(accounts);
        self.loans = Some(loans);
        self.audit = Some(audit);
        self
    }

    // ── Joined helpers for the dashboard (people, not ids) ───────────────

    async fn recent_transfers_detailed(&self, limit: i64) -> Result<Vec<AdminTransferRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminTransferRow>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id,
                   fa.user_id AS from_user_id, ta.user_id AS to_user_id,
                   fa.account_number AS from_number, ta.account_number AS to_number,
                   fu.full_name AS from_owner, tu.full_name AS to_owner,
                   t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            ORDER BY t.created_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    /// Rule-based fraud signals, with the reason computed in SQL so aggregate
    /// rules (velocity) can be explained alongside per-row ones. Rules:
    ///   1. rejected transfers (except insufficient funds - that's pre-checked
    ///      before anything is posted, so it isn't a fraud signal),
    ///   2. anything currently on hold (with its stored reason),
    ///   3. large transfers (>= $10,000),
    ///   4. velocity - the source account made 4+ transfers in the prior hour.
    async fn flagged_transfers_detailed(&self) -> Result<Vec<FlaggedTransfer>, AppError> {
        let rows = sqlx::query_as::<_, FlaggedTransfer>(
            r#"
            SELECT t.id,
                   fa.user_id AS from_user_id, ta.user_id AS to_user_id,
                   fu.full_name AS from_owner, tu.full_name AS to_owner,
                   t.amount, t.status,
                   CASE
                     WHEN t.status = 'on_hold'
                          THEN COALESCE(t.status_reason, 'On hold for review')
                     WHEN t.status = 'rejected'
                          THEN COALESCE(t.status_reason, 'Rejected transfer')
                     WHEN t.amount >= $1
                          THEN 'Large transfer (>= $10,000)'
                     ELSE 'High velocity: 4+ transfers from this account within 1 hour'
                   END AS reason,
                   t.created_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            WHERE (t.status = 'rejected'
                   AND (t.status_reason IS NULL OR t.status_reason NOT LIKE 'insufficient funds%'))
               OR t.status = 'on_hold'
               OR t.amount >= $1
               OR (
                    SELECT COUNT(*) FROM transfers v
                    WHERE v.from_account_id = t.from_account_id
                      AND v.created_at <= t.created_at
                      AND v.created_at >  t.created_at - INTERVAL '1 hour'
                  ) >= 4
            ORDER BY t.created_at DESC
            LIMIT 100
            "#,
        )
        .bind(Decimal::from(LARGE_TRANSFER_THRESHOLD))
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn pending_loans_detailed(&self) -> Result<Vec<AdminLoanRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminLoanRow>(
            r#"
            SELECT l.id, l.user_id, u.full_name AS applicant_name,
                   l.principal, l.interest_rate, l.term_months
            FROM loans l
            JOIN users u ON u.id = l.user_id
            WHERE l.status = 'pending'
            ORDER BY l.created_at ASC
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}

#[async_trait]
impl AdminService for PgAdminService {
    async fn snapshot(&self) -> Result<DashboardSnapshot, AppError> {
        // Use cheap defaults if injection hasn't happened yet - keeps the
        // dashboard renderable during scaffolding.
        let active_accounts = match &self.accounts {
            Some(a) => a.count_active().await?,
            None => 0,
        };
        let total_deposits = match &self.accounts {
            Some(a) => a.total_deposits().await?,
            None => Decimal::ZERO,
        };
        let portfolio_outstanding = match &self.loans {
            Some(l) => l.portfolio_outstanding().await?,
            None => Decimal::ZERO,
        };
        // Joined directly so the dashboard shows applicant/owner names, not ids.
        let pending_loans_list = self.pending_loans_detailed().await?;
        let recent_transfers = self.recent_transfers_detailed(10).await?;
        let flagged_transfers = self.flagged_transfers_detailed().await?;
        let recent_audit = match &self.audit {
            Some(a) => a.recent(20).await?,
            None => vec![],
        };

        Ok(DashboardSnapshot {
            active_accounts,
            total_deposits,
            portfolio_outstanding,
            pending_loans: pending_loans_list.len(),
            recent_transfers,
            flagged_transfers,
            pending_loans_list,
            recent_audit,
        })
    }

    async fn all_accounts(&self) -> Result<Vec<AdminAccountRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminAccountRow>(
            r#"
            SELECT a.id, a.user_id, a.account_number, a.kind, a.status, a.balance, a.created_at,
                   u.full_name AS owner_name, u.email AS owner_email
            FROM accounts a
            JOIN users u ON u.id = a.user_id
            ORDER BY a.created_at DESC
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn all_transfers(
        &self,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    ) -> Result<Vec<AdminTransferRow>, AppError> {
        // Every transfer in the bank, optionally bounded by an inclusive date
        // range. The `$n::date IS NULL` guards make each bound optional, and the
        // upper bound adds a day so the whole `to` date is included.
        let rows = sqlx::query_as::<_, AdminTransferRow>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id,
                   fa.user_id AS from_user_id, ta.user_id AS to_user_id,
                   fa.account_number AS from_number, ta.account_number AS to_number,
                   fu.full_name AS from_owner, tu.full_name AS to_owner,
                   t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            WHERE ($1::date IS NULL OR t.created_at >= $1::date)
              AND ($2::date IS NULL OR t.created_at <  ($2::date + INTERVAL '1 day'))
            ORDER BY t.created_at DESC
            "#,
        )
        .bind(from)
        .bind(to)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn transfers_for_user(&self, user_id: i64) -> Result<Vec<AdminTransferRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminTransferRow>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id,
                   fa.user_id AS from_user_id, ta.user_id AS to_user_id,
                   fa.account_number AS from_number, ta.account_number AS to_number,
                   fu.full_name AS from_owner, tu.full_name AS to_owner,
                   t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            WHERE fa.user_id = $1 OR ta.user_id = $1
            ORDER BY t.created_at DESC
            LIMIT 200
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn customers(&self) -> Result<Vec<AdminUserRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminUserRow>(
            r#"
            SELECT id, email, full_name, role
            FROM users
            WHERE role = 'customer'
            ORDER BY full_name
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}
