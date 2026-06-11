//! Transfer model - owned by the Transfers module (Member 4).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "transfer_status", rename_all = "snake_case")]
pub enum TransferStatus {
    /// Created but waiting on OTP confirmation.
    Pending,
    /// Money has moved, audit row written.
    Completed,
    /// Failed for a technical reason (DB error, etc.).
    Failed,
    /// Refused for a business reason (insufficient funds, frozen account, fraud rule).
    Rejected,
    /// Fraud rules tripped at confirm time. Money has NOT moved; the customer
    /// submits a review request and staff release or deny it.
    OnHold,
}

impl TransferStatus {
    pub fn label(&self) -> &'static str {
        match self {
            TransferStatus::Pending => "Pending",
            TransferStatus::Completed => "Completed",
            TransferStatus::Failed => "Failed",
            TransferStatus::Rejected => "Rejected",
            TransferStatus::OnHold => "On hold",
        }
    }

    /// Tailwind classes for a status pill (used by the admin tables).
    pub fn badge(&self) -> &'static str {
        match self {
            TransferStatus::Completed => "bg-emerald-50 text-emerald-700 border-emerald-200",
            TransferStatus::Pending => "bg-amber-50 text-amber-700 border-amber-200",
            TransferStatus::Failed => "bg-stone-100 text-stone-600 border-stone-200",
            TransferStatus::Rejected => "bg-red-50 text-red-700 border-red-200",
            TransferStatus::OnHold => "bg-purple-50 text-purple-700 border-purple-200",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct Transfer {
    pub id: i64,
    pub from_account_id: i64,
    pub to_account_id: i64,
    pub amount: Decimal,
    pub status: TransferStatus,
    pub note: Option<String>,
    /// Human-readable reason a transfer was rejected or flagged (e.g.
    /// "insufficient funds"). `None` for ordinary completed/pending transfers.
    pub status_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
