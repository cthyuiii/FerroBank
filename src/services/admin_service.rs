//! Admin service — owned by the Platform Lead (Member 1).
//!
//! Composes read-only data from every other module into the dashboard view-model.
//! Holds Arcs of the other service traits so it's testable with mocks.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use sqlx::PgPool;

use crate::errors::AppError;
use crate::models::loan::Loan;
use crate::models::transfer::Transfer;
use crate::services::account_service::AccountService;
use crate::services::audit_service::{AuditEntry, AuditService};
use crate::services::loan_service::LoanService;
use crate::services::transfer_service::TransferService;

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
}
