//! Dev-only seed script: creates demo users, accounts, loans, transfers, and
//! audit entries so the dashboards have something to show without anyone having
//! to click through the UI first.
//!
//! Usage:   cargo run --bin seed
//!
//! **Idempotent.** Re-running skips rows that already exist, so it's safe to run
//! repeatedly. Direct SQL inserts are used for "historical" transfers and audit
//! entries because the production code paths (TransferService::create + confirm,
//! AuditService::record) all stamp `created_at = now()` and we want backdated
//! demo data.

use std::collections::HashMap;

use rust_decimal::Decimal;
use sqlx::PgPool;

use ferrobank::{
    config::Config,
    db,
    errors::AppError,
    models::account::AccountType,
    models::user::{NewUser, Role},
    services::account_service::{AccountService, PgAccountService},
    services::auth_service::{AuthService, PgAuthService},
    services::loan_service::{LoanService, PgLoanService},
};

// ── User seed plan ──────────────────────────────────────────────────

struct SeedUser {
    key: &'static str,
    email: &'static str,
    password: &'static str,
    full_name: &'static str,
    role: Role,
}

const SEED_USERS: &[SeedUser] = &[
    SeedUser { key: "admin",   email: "admin@ferrobank.local",   password: "admin123",   full_name: "Admin User",       role: Role::Admin    },
    SeedUser { key: "teller",  email: "teller@ferrobank.local",  password: "teller123",  full_name: "Teller User",      role: Role::Teller   },
    SeedUser { key: "alice",   email: "alice@ferrobank.local",   password: "alice123",   full_name: "Alice Smith",      role: Role::Customer },
    SeedUser { key: "bob",     email: "bob@ferrobank.local",     password: "bob123",     full_name: "Bob Johnson",      role: Role::Customer },
    SeedUser { key: "charlie", email: "charlie@ferrobank.local", password: "charlie123", full_name: "Charlie Williams", role: Role::Customer },
    SeedUser { key: "diana",   email: "diana@ferrobank.local",   password: "diana123",   full_name: "Diana Brown",      role: Role::Customer },
];

// ── Account seed plan ───────────────────────────────────────────────

struct SeedAccount {
    key: &'static str,
    owner: &'static str,
    kind: AccountType,
    initial_balance_cents: i64,
}

const SEED_ACCOUNTS: &[SeedAccount] = &[
    SeedAccount { key: "alice_savings",    owner: "alice",   kind: AccountType::Savings,  initial_balance_cents: 500_000 },
    SeedAccount { key: "alice_checking",   owner: "alice",   kind: AccountType::Checking, initial_balance_cents: 250_000 },
    SeedAccount { key: "bob_savings",      owner: "bob",     kind: AccountType::Savings,  initial_balance_cents: 120_000 },
    SeedAccount { key: "bob_checking",     owner: "bob",     kind: AccountType::Checking, initial_balance_cents:  80_000 },
    SeedAccount { key: "charlie_checking", owner: "charlie", kind: AccountType::Checking, initial_balance_cents:  35_000 },
    SeedAccount { key: "diana_savings",    owner: "diana",   kind: AccountType::Savings,  initial_balance_cents:      0 },
];

// ── main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;

    // Make the seed self-sufficient: apply migrations before inserting anything,
    // so it can run on its own (e.g. as part of `docker compose up`) without
    // waiting for the app container to create the schema first. sqlx takes a
    // migration advisory lock, so running this alongside the app's own
    // migration step is safe — whichever acquires the lock first applies them,
    // the other sees them already applied and no-ops.
    println!("── Migrations ────────────────────────────────");
    sqlx::migrate!("./migrations").run(&pool).await?;
    println!("  ✓ schema up to date");

    let auth = PgAuthService::new(pool.clone());
    let accounts = PgAccountService::new(pool.clone());
    let loans = PgLoanService::new(pool.clone());

    println!("── Users ─────────────────────────────────────");
    let user_ids = seed_users(&auth, &pool).await?;

    println!("\n── Accounts ──────────────────────────────────");
    let account_ids = seed_accounts(&accounts, &user_ids, &pool).await?;

    println!("\n── Loans ─────────────────────────────────────");
    seed_loans(&loans, &user_ids).await?;

    println!("\n── Historical transfers ──────────────────────");
    seed_transfers(&account_ids, &pool).await?;

    println!("\n── Audit log ─────────────────────────────────");
    seed_audit(&user_ids, &pool).await?;

    println!("\nDone. Sign in at http://localhost:8080/login");
    println!("  admin@ferrobank.local   / admin123");
    println!("  teller@ferrobank.local  / teller123");
    println!("  alice@ferrobank.local   / alice123");
    println!("  (others: bob, charlie, diana — same pattern)");
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn seed_users(
    auth: &PgAuthService,
    pool: &PgPool,
) -> anyhow::Result<HashMap<&'static str, i64>> {
    let mut ids = HashMap::new();
    for u in SEED_USERS {
        let new = NewUser {
            email: u.email.to_string(),
            password: u.password.to_string(),
            full_name: u.full_name.to_string(),
            role: u.role,
        };
        match auth.register(new).await {
            Ok(user) => {
                println!("  ✓ created {:<28} ({:?})  id={}", u.email, u.role, user.id);
                ids.insert(u.key, user.id);
            }
            Err(AppError::Conflict(_)) => {
                // Already exists — look up the id by email (avoids re-hashing the password).
                let row: (i64,) =
                    sqlx::query_as("SELECT id FROM users WHERE email = $1")
                        .bind(u.email)
                        .fetch_one(pool)
                        .await?;
                println!("  · exists  {:<28} ({:?})  id={}", u.email, u.role, row.0);
                ids.insert(u.key, row.0);
            }
            Err(e) => return Err(anyhow::anyhow!("user {}: {e}", u.email)),
        }
    }
    Ok(ids)
}

async fn seed_accounts(
    accounts: &PgAccountService,
    user_ids: &HashMap<&'static str, i64>,
    pool: &PgPool,
) -> anyhow::Result<HashMap<&'static str, i64>> {
    let mut ids = HashMap::new();
    for a in SEED_ACCOUNTS {
        let user_id = user_ids[a.owner];

        // Reuse an existing account of the same kind for this user, if any.
        let existing = accounts.list_for_user(user_id).await?;
        let account_id = match existing.iter().find(|x| x.kind == a.kind) {
            Some(x) => {
                println!("  · exists  {:<22} kind={:?}  id={}", a.key, a.kind, x.id);
                x.id
            }
            None => {
                let x = accounts.open_account(user_id, a.kind).await?;
                println!("  ✓ created {:<22} kind={:?}  id={}", a.key, a.kind, x.id);
                x.id
            }
        };

        // Set the starting balance directly. Bypassing the transfer flow is
        // fine here because this is the dev seed; production never SETs a
        // balance, it only moves money between accounts.
        let balance = Decimal::new(a.initial_balance_cents, 2);
        sqlx::query("UPDATE accounts SET balance = $1 WHERE id = $2")
            .bind(balance)
            .bind(account_id)
            .execute(pool)
            .await?;

        ids.insert(a.key, account_id);
    }
    Ok(ids)
}

async fn seed_loans(
    loans: &PgLoanService,
    user_ids: &HashMap<&'static str, i64>,
) -> anyhow::Result<()> {
    // (owner, principal, rate, term_months, approve?)
    let plan: &[(&str, &str, &str, i32, bool)] = &[
        ("alice",   "10000.00", "0.0525", 36, false),
        ("bob",      "5000.00", "0.0700", 24, true ),  // approved
        ("charlie",  "2500.00", "0.0650", 12, false),
    ];

    for (owner, principal, rate, term, approve) in plan {
        let p: Decimal = principal.parse()?;
        let r: Decimal = rate.parse()?;
        match loans.apply(user_ids[*owner], p, r, *term).await {
            Ok(loan) => {
                println!("  ✓ {} applied  id={} ${} @ {} ({} mo)", owner, loan.id, p, r, term);
                if *approve {
                    // A loan now needs BOTH a teller and an admin approval, so
                    // the seed records one of each to fully approve it.
                    loans.approve(loan.id, user_ids["teller"], Role::Teller).await?;
                    loans.approve(loan.id, user_ids["admin"], Role::Admin).await?;
                    println!("    └ approved (teller + admin)");
                }
            }
            Err(AppError::Conflict(msg)) => {
                println!("  · {} loan conflict: {msg}", owner);
            }
            Err(e) => return Err(anyhow::anyhow!("loan for {owner}: {e}")),
        }
    }
    Ok(())
}

async fn seed_transfers(
    account_ids: &HashMap<&'static str, i64>,
    pool: &PgPool,
) -> anyhow::Result<()> {
    // If any transfers already exist, leave them alone — keeps the script idempotent
    // without us having to track individual rows.
    let existing: (i64,) =
        sqlx::query_as("SELECT COUNT(*)::BIGINT FROM transfers")
            .fetch_one(pool)
            .await?;
    if existing.0 > 0 {
        println!("  · {} transfers already present, skipping", existing.0);
        return Ok(());
    }

    // (from, to, amount, status, note, days_ago)
    let plan: &[(&str, &str, &str, &str, &str, i32)] = &[
        ("alice_checking",  "bob_checking",     "100.00",  "completed", "rent split",   7),
        ("bob_checking",    "charlie_checking",  "50.00",  "completed", "groceries",    1),
        ("alice_savings",   "diana_savings",    "200.00",  "completed", "welcome gift", 0),
        ("alice_checking",  "bob_savings",     "9999.00",  "completed", "wedding gift", 2),  // flagged: ≥ $10k threshold? — under, but big
        ("bob_checking",    "alice_checking",    "25.00",  "rejected",  "test",         3),
    ];

    for (from_key, to_key, amount, status, note, days_ago) in plan {
        let from = account_ids[*from_key];
        let to = account_ids[*to_key];
        let amount: Decimal = amount.parse()?;
        sqlx::query(
            r#"
            INSERT INTO transfers
                (from_account_id, to_account_id, amount, status, note, created_at)
            VALUES
                ($1, $2, $3, $4::transfer_status, $5, now() - ($6 || ' days')::interval)
            "#,
        )
        .bind(from)
        .bind(to)
        .bind(amount)
        .bind(status)
        .bind(note)
        .bind(days_ago.to_string())
        .execute(pool)
        .await?;
        println!("  ✓ {} → {}  ${} ({})", from_key, to_key, amount, status);
    }
    Ok(())
}

async fn seed_audit(
    user_ids: &HashMap<&'static str, i64>,
    pool: &PgPool,
) -> anyhow::Result<()> {
    let existing: (i64,) =
        sqlx::query_as("SELECT COUNT(*)::BIGINT FROM audit_log")
            .fetch_one(pool)
            .await?;
    if existing.0 > 0 {
        println!("  · {} audit entries already present, skipping", existing.0);
        return Ok(());
    }

    // (actor_key, event, payload_json, hours_ago)
    let plan: &[(&str, &str, &str, i32)] = &[
        ("admin",  "auth.login.success",   r#"{"ip":"127.0.0.1"}"#,                     1),
        ("alice",  "auth.login.success",   r#"{"ip":"127.0.0.1"}"#,                     2),
        ("alice",  "account.opened",       r#"{"kind":"savings"}"#,                   720),
        ("bob",    "transfer.completed",   r#"{"amount":"100.00","to_user":"alice"}"#, 168),
        ("teller", "account.frozen",       r#"{"account_id":42,"reason":"review"}"#,    5),
        ("admin",  "loan.approved",        r#"{"loan_id":2,"principal":"5000.00"}"#,    3),
    ];

    for (actor, event, payload, hours_ago) in plan {
        let actor_id = user_ids[*actor];
        sqlx::query(
            r#"
            INSERT INTO audit_log (actor_user_id, event, payload, created_at)
            VALUES ($1, $2, $3::jsonb, now() - ($4 || ' hours')::interval)
            "#,
        )
        .bind(actor_id)
        .bind(event)
        .bind(payload)
        .bind(hours_ago.to_string())
        .execute(pool)
        .await?;
        println!("  ✓ {:<22} by {}", event, actor);
    }
    Ok(())
}
