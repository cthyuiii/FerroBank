//! Integration tests: the transfer engine under concurrent load.
//!
//! Together these prove the transfer engine's core guarantees:
//!
//!   - prevents race conditions          (locks serialize the balance checks)
//!   - prevents inconsistent balances    (sum of money is conserved, never negative)
//!   - prevents double spending          (the same transfer can never pay out twice)
//!
//! Needs a live Postgres. With the dev database running (docker compose or a
//! native install):
//!
//!     cargo test --test transfer_concurrency -- --nocapture
//!
//! When DATABASE_URL is not set, each test skips itself so a plain
//! `cargo test` still passes everywhere.

use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::PgPool;

use ferrobank::models::account::AccountType;
use ferrobank::models::user::{NewUser, Role};
use ferrobank::services::account_service::{AccountService, PgAccountService};
use ferrobank::services::audit_service::PgAuditService;
use ferrobank::services::auth_service::{AuthService, PgAuthService};
use ferrobank::services::telegram_service::ScreenOtp;
use ferrobank::services::transfer_service::{PgTransferService, TransferService};

/// Everything a test needs: two fresh customers, a funded source account, an
/// empty destination account, and the services. Returns `None` (→ test skips)
/// when DATABASE_URL is unset.
struct Rig {
    pool: PgPool,
    sender_id: i64,
    from_id: i64,
    to_id: i64,
    accounts: PgAccountService,
    transfers: Arc<dyn TransferService>,
}

async fn rig(fund_dollars: i64) -> Option<Rig> {
    dotenvy::dotenv().ok();
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set (start Postgres and re-run)");
        return None;
    };

    let pool = ferrobank::db::connect(&url).await.expect("db connect");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrations");

    // Fresh actors every run - unique emails keep the tests idempotent.
    let stamp = chrono::Utc::now().timestamp_micros();
    let auth = PgAuthService::new(pool.clone());
    let sender = auth
        .register(NewUser {
            email: format!("test+sender{stamp}@test.local"),
            password: "password123".into(),
            first_name: "Concurrency".into(),
            middle_name: None,
            last_name: "Sender".into(),
            nric: Some("T0000001A".into()),
            role: Role::Customer,
        })
        .await
        .expect("register sender");
    let recipient = auth
        .register(NewUser {
            email: format!("test+recipient{stamp}@test.local"),
            password: "password123".into(),
            first_name: "Concurrency".into(),
            middle_name: None,
            last_name: "Recipient".into(),
            nric: Some("T0000002B".into()),
            role: Role::Customer,
        })
        .await
        .expect("register recipient");

    let accounts = PgAccountService::new(pool.clone(), Arc::new(ScreenOtp));
    let from = accounts
        .open_account(sender.id, AccountType::Checking, true)
        .await
        .expect("open sender account");
    let to = accounts
        .open_account(recipient.id, AccountType::Checking, true)
        .await
        .expect("open recipient account");
    accounts
        .adjust_balance(from.id, Decimal::from(fund_dollars))
        .await
        .expect("fund sender account");

    let audit = Arc::new(PgAuditService::new(pool.clone()));
    let transfers: Arc<dyn TransferService> =
        Arc::new(PgTransferService::new(pool.clone(), audit, Arc::new(ScreenOtp)));

    Some(Rig {
        pool,
        sender_id: sender.id,
        from_id: from.id,
        to_id: to.id,
        accounts,
        transfers,
    })
}

/// Race condition + inconsistent balance: five simultaneous $10 transfers out
/// of a $30 balance. Without the `FOR UPDATE` row locks, several tasks would
/// read "balance = 30" at once and all debit - overdrawing the account. With
/// the locks, exactly three fit and every dollar is accounted for.
#[tokio::test]
async fn concurrent_transfers_never_overdraw_or_lose_money() {
    let Some(rig) = rig(30).await else { return };

    let mut handles = Vec::new();
    for _ in 0..5 {
        let svc = rig.transfers.clone();
        let (actor, from_id, to_id) = (rig.sender_id, rig.from_id, rig.to_id);
        handles.push(tokio::spawn(async move {
            let created = svc
                .create(actor, from_id, to_id, Decimal::from(10), None)
                .await?;
            svc.confirm(actor, created.transfer.id, &created.otp)
                .await
                .map(|_| ())
        }));
    }

    let mut completed = 0usize;
    let mut rejected = 0usize;
    for h in handles {
        match h.await.expect("transfer task panicked") {
            Ok(()) => completed += 1,
            Err(_) => rejected += 1,
        }
    }

    let from_balance = rig.accounts.get_balance(rig.from_id).await.unwrap();
    let to_balance = rig.accounts.get_balance(rig.to_id).await.unwrap();

    assert_eq!(completed, 3, "exactly three $10 transfers fit in $30");
    assert_eq!(rejected, 2, "the two losers must be rejected, not lost");
    assert_eq!(from_balance, Decimal::ZERO, "balance never goes negative");
    assert_eq!(
        to_balance,
        Decimal::from(30),
        "every dollar debited arrived - none lost, none duplicated"
    );
}

/// Double spending: ONE pending transfer, confirmed twice at the same instant
/// (e.g. a double-clicked submit button, or a replayed request). The row lock
/// on the transfer serializes the two confirms; the loser sees status !=
/// 'pending' and is refused, so the money moves exactly once.
#[tokio::test]
async fn confirming_the_same_transfer_twice_moves_money_exactly_once() {
    let Some(rig) = rig(50).await else { return };

    let created = rig
        .transfers
        .create(rig.sender_id, rig.from_id, rig.to_id, Decimal::from(20), None)
        .await
        .expect("create transfer");

    let (svc1, svc2) = (rig.transfers.clone(), rig.transfers.clone());
    let (otp1, otp2) = (created.otp.clone(), created.otp.clone());
    let (actor, transfer_id) = (rig.sender_id, created.transfer.id);

    let (r1, r2) = tokio::join!(
        tokio::spawn(async move { svc1.confirm(actor, transfer_id, &otp1).await.map(|_| ()) }),
        tokio::spawn(async move { svc2.confirm(actor, transfer_id, &otp2).await.map(|_| ()) }),
    );
    let results = [r1.expect("task 1 panicked"), r2.expect("task 2 panicked")];
    let successes = results.iter().filter(|r| r.is_ok()).count();

    let from_balance = rig.accounts.get_balance(rig.from_id).await.unwrap();
    let to_balance = rig.accounts.get_balance(rig.to_id).await.unwrap();

    assert_eq!(successes, 1, "a transfer must pay out exactly once");
    assert_eq!(from_balance, Decimal::from(30), "debited once, not twice");
    assert_eq!(to_balance, Decimal::from(20), "credited once, not twice");

    // And the transfer row itself ended as 'completed' (not re-processable).
    let status: (String,) =
        sqlx::query_as(r#"SELECT status::TEXT FROM transfers WHERE id = $1"#)
            .bind(transfer_id)
            .fetch_one(&rig.pool)
            .await
            .unwrap();
    assert_eq!(status.0, "completed");
}
