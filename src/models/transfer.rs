//! Transfer model — owned by the Transfers module (Member 4).

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
}

impl TransferStatus {
    pub fn label(&self) -> &'static str {
        match self {
            TransferStatus::Pending => "Pending",
            TransferStatus::Completed => "Completed",
            TransferStatus::Failed => "Failed",
            TransferStatus::Rejected => "Rejected",
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
    pub created_at: chrono::DateTime<chrono::Utc>,
}
