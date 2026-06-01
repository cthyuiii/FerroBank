//! Admin service — owned by the Platform Lead (Member 1).
//!
//! Composes read-only data from every other module into the dashboard view-model.
//! Holds Arcs of the other service traits so it's testable with mocks.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool};

use crate::errors::AppError;
use crate::models::account::{AccountStatus, AccountType};
use crate::models::loan::Loan;
use crate::models::transfer::{Transfer, TransferStatus};
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::audit_service::{AuditEntry, AuditService};
use crate::services::loan_service::LoanService;
use crate::services::transfer_service::TransferService;

/// Threshold (dollars) at or above which a transfer is treated as noteworthy.
const LARGE_TRANSFER_THRESHOLD: i64 = 10_000;

/// One row of the admin "all accounts" table — account joined to its owner.
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

/// One row of the admin "all transfers" table — transfer joined to both owners.
#[derive(Debug, Clone, FromRow)]
pub struct AdminTransferRow {
    pub id: i64,
    pub from_account_id: i64,
    pub to_account_id: i64,
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
            return Some("Large transfer (≥ $10,000)".to_string());
        }
        None
    }
}

/// One row of the admin users picker — used to open an account on someone's behalf.
#[derive(Debug, Clone, FromRow)]
pub struct AdminUserRow {
    pub id: i64,
    pub email: String,
    pub full_name: String,
    pub role: Role,
}

/// Pre-computed snapshot shown on `/admin/dashboard`.
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub active_accounts: i64,
    pub total_deposits: Decimal,
    pub portfolio_outstanding: Decimal,
    pub pending_loans: usize,
    pub recent_transfers: Vec<Transfer>,
    pub flagged_transfers: Vec<Transfer>,
    pub pending_loans_list: Vec<Loan>,
    pub recent_audit: Vec<AuditEntry>,
}

#[async_trait]
pub trait AdminService: Send + Sync {
    async fn snapshot(&self) -> Result<DashboardSnapshot, AppError>;

    /// Every account in the bank, joined to its owner, newest first.
    async fn all_accounts(&self) -> Result<Vec<AdminAccountRow>, AppError>;
    /// Every transfer, joined to both parties, newest first (capped).
    async fn all_transfers(&self) -> Result<Vec<AdminTransferRow>, AppError>;
    /// All users — for the "open an account for a customer" picker.
    async fn all_users(&self) -> Result<Vec<AdminUserRow>, AppError>;
}

pub struct PgAdminService {
    pub db: PgPool,
    // The other services are set after construction via `with_services`.
    // We can't take them in `new` because `PgAdminService` itself is constructed
    // in `main.rs` before the other services exist (chicken-and-egg avoidance).
    // The Platform Lead injects them via a builder method instead.
    pub accounts: Option<Arc<dyn AccountService>>,
    pub transfers: Option<Arc<dyn TransferService>>,
    pub loans: Option<Arc<dyn LoanService>>,
    pub audit: Option<Arc<dyn AuditService>>,
}

impl PgAdminService {
    pub fn new(db: PgPool) -> Self {
        Self {
            db,
            accounts: None,
            transfers: None,
            loans: None,
            audit: None,
        }
    }

    /// Inject the dependent services. Called by main.rs after all services are built.
    pub fn with_services(
        mut self,
        accounts: Arc<dyn AccountService>,
        transfers: Arc<dyn TransferService>,
        loans: Arc<dyn LoanService>,
        audit: Arc<dyn AuditService>,
    ) -> Self {
        self.accounts = Some(accounts);
        self.transfers = Some(transfers);
        self.loans = Some(loans);
        self.audit = Some(audit);
        self
    }
}

#[async_trait]
impl AdminService for PgAdminService {
    async fn snapshot(&self) -> Result<DashboardSnapshot, AppError> {
        // Use cheap defaults if injection hasn't happened yet — keeps the
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
        let pending_loans_list = match &self.loans {
            Some(l) => l.pending_applications().await?,
            None => vec![],
        };
        let recent_transfers = match &self.transfers {
            Some(t) => t.recent(10).await?,
            None => vec![],
        };
        let flagged_transfers = match &self.transfers {
            Some(t) => t.flagged().await?,
            None => vec![],
        };
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

    async fn all_transfers(&self) -> Result<Vec<AdminTransferRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminTransferRow>(
            r#"
            SELECT t.id, t.from_account_id, t.to_account_id,
                   fa.account_number AS from_number, ta.account_number AS to_number,
                   fu.full_name AS from_owner, tu.full_name AS to_owner,
                   t.amount, t.status, t.note, t.status_reason, t.created_at
            FROM transfers t
            JOIN accounts fa ON fa.id = t.from_account_id
            JOIN accounts ta ON ta.id = t.to_account_id
            JOIN users    fu ON fu.id = fa.user_id
            JOIN users    tu ON tu.id = ta.user_id
            ORDER BY t.created_at DESC
            LIMIT 500
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }

    async fn all_users(&self) -> Result<Vec<AdminUserRow>, AppError> {
        let rows = sqlx::query_as::<_, AdminUserRow>(
            r#"
            SELECT id, email, full_name, role
            FROM users
            ORDER BY id
            "#,
        )
        .fetch_all(&self.db)
        .await?;
        Ok(rows)
    }
}
