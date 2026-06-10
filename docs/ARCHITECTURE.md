# FerroBank — Architecture

How the pieces fit together. See **[docs/uml_domain_model.mermaid](./docs/uml_domain_model.mermaid)**
and **[docs/uml_service_architecture.mermaid](./docs/uml_service_architecture.mermaid)** for the
class diagrams, and **[PROJECT_FILE_GUIDE.md](./PROJECT_FILE_GUIDE.md)** for a file-by-file tour.

---

## High-level request flow

```
Browser
  ↓ HTTP / form POST
[ Actix middleware ]   ← session cookie, CurrentUser extractor, RequireRole guard, logging
  ↓
[ Handler (Actix route) ]   ← parses form, calls a service, picks a template
  ↓                              ↘
[ Service trait (dyn) ]            [ Askama template ]  → HTML
  ↓
[ Model + SQLx query ]
  ↓
PostgreSQL
```

Handlers are **thin** (parse input, call a service, render). Services are **thick** —
all business rules and transactions live there. Models are SQLx-typed structs.

---

## Layered + OOP design (how the spec's OOP requirements are met)

The OOP design centres on four concepts: **encapsulation, traits, polymorphism, and
concurrency safety (Arc/Mutex)**. Mapping each to the code:

| Concept | Where it lives |
|---|---|
| **Encapsulation** | Each `Pg…Service` keeps its fields **private** (`db`, `audit`, `rate_limit`, the admin's injected services). Callers only touch the trait methods — they know nothing about SQL or the pool. Internal-only details (`otp_hash`, the `PendingRow`, `hash_password`) are never exposed on public types. |
| **Traits (abstraction / shared interface)** | Six service traits — `AuthService`, `AccountService`, `TransferService`, `LoanService`, `AuditService`, `AdminService` — define behaviour; `Pg…` structs implement it. Traits as shared interfaces — the classic Rust abstraction pattern. |
| **Polymorphism** | *Dynamic dispatch:* services are used as `Arc<dyn Trait>` / `web::Data<dyn Trait>`, so handlers depend on the abstraction and an impl is swappable (e.g. a mock in tests). *Parametric:* `fn render<T: Template>(t: T)`. *Ad-hoc:* enums (`Role`, `AccountStatus`, …) carry behaviour via `impl` (`label()`, `badge()`). |
| **Inheritance (Rust-style)** | Traits use **supertraits** (`pub trait TransferService: Send + Sync`). Rust has no class inheritance; trait composition + the `AdminService` *composing* the other services (`with_services(...)`) is the idiomatic substitute. |
| **Concurrency (Arc + Mutex)** | The transfer engine holds an `Arc`-shared, `tokio::sync::Mutex`-guarded rate-limit map, layered on top of database row locks (below). |

### A note on subtype-style account modelling

A common illustration of polymorphism uses `SavingsAccount` / `CurrentAccount` /
`BusinessAccount` structs each implementing a `BankAccount` trait. FerroBank models
account variety with an **`AccountType` enum** plus behaviour in `AccountService`
(layered architecture), and gets its polymorphism at the **service layer** via
`dyn` traits instead. Both are valid OOP; the difference is *where* the polymorphism
sits. (If subtype-style polymorphism is wanted explicitly, a small `AccountKind`
trait implemented by per-type structs can be added — see the README's roadmap.)

### Mapping to the four core banking objects

| Core object | FerroBank realisation |
|---|---|
| `BankAccount` (id, owner, balance, status) | `models::account::Account` + `AccountService` (open/approve/freeze/close/adjust/balance) |
| `MoneyTransfer` (from, to, amount, status, timestamp) | `models::transfer::Transfer` + `TransferService` (create → confirm) |
| `TransferEngine` (accounts, logs, rules) | `PgTransferService` — the engine: Mutex rate-limiter + `FOR UPDATE` row locks + fraud/business rules + audit |
| `AuditLog` (transfer_id, action, timestamp, result) | `models`/`AuditService` `audit_log` table, written on every state change |

---

## Concurrency layers

The banking domain must show **both** database- and application-level concurrency control.

| Layer | Primitive | Protects against |
|---|---|---|
| Database | `SELECT … FOR UPDATE` inside a `BEGIN/COMMIT` transaction; explicit `rollback()` on failure | Lost updates, partial money moves, double-spend, negative balances |
| Application (Rust) | `tokio::sync::Mutex<HashMap<…>>` behind `Arc` | Per-account rate-limit bursts before they reach the DB |

**Lock ordering:** account rows are always locked in ascending `id` order to avoid
deadlocks when two transfers touch the same pair in opposite directions.

**Atomicity is explicit:** in `TransferService::confirm`, the debit + credit + finalize
run through `apply_money_move`; on any error the code calls `tx.rollback()` and returns,
so a partial debit can never be committed.

---

## Key domain workflows

- **Money transfer (two-step + OTP):** `create` validates, rate-limits (Mutex), generates a
  6-digit OTP (argon2-hashed in the DB), inserts a `pending` row. `confirm` verifies the OTP,
  locks both accounts `FOR UPDATE`, re-checks status/balance, moves money in one transaction,
  audits. Rejections store a human-readable `status_reason`.
- **Account approval:** a customer self-opening creates a `pending` account; a **teller or admin**
  approves it to `active` (`/staff/accounts`). Admin-opened accounts are active immediately.
  A DB trigger enforces that only **customers** may own accounts.
- **Loan dual approval:** a pending loan needs **one teller approval AND one admin approval**
  (two distinct roles, tracked in `loan_approvals`) before it becomes `approved`.
- **Loan repayment:** debits a chosen funding account inside the same transaction that records
  the repayment; refuses if the account is inactive or underfunded.
- **Fraud signals (dashboard):** rejected transfers, large transfers (≥ $10k), structuring
  (≥ $9k, just under threshold), and velocity (4+ transfers from one account in 24h).

---

## Roles & access (RBAC)

| Area | Guard | Who |
|---|---|---|
| `/accounts`, `/transfers`, `/loans` (customer views) | `CurrentUser` | logged-in users |
| `/admin/*` (dashboard, audit, account mutations) | `RequireRole(Admin)` | admin only |
| `/staff/*` (all accounts + approve, all transfers) | `RequireRole(Teller)` | teller **and** admin (admin is a superuser in the guard) |

Post-login routing: admin → `/admin/dashboard`, teller → `/loans`, customer → `/accounts`.

---

## Stable surface (the platform contract)

### `AppState` — `web::Data<AppState>`
```rust
pub struct AppState { pub db: PgPool, pub config: Arc<Config> }
```

### `AppError` — one error type, implements `ResponseError`
`NotFound | Unauthorized | Forbidden | BadRequest | Conflict | Internal`. `?` maps it to the
right HTTP status (and `Unauthorized` redirects to `/login`). Has `From<sqlx::Error>` etc.

### `CurrentUser` — extractor
Drop into a handler to require login; use `Option<CurrentUser>` for "maybe logged in" (e.g. the
landing page). Missing/invalid session → `AppError::Unauthorized` → redirect to `/login`.

### `RequireRole(Role)` — scope guard, applied with `.wrap(...)`
```rust
web::scope("/admin").wrap(RequireRole(Role::Admin)) // admin only
web::scope("/staff").wrap(RequireRole(Role::Teller)) // teller + admin
```

---

## Service pattern (the OOP core)

```rust
#[async_trait::async_trait]
pub trait AccountService: Send + Sync {
    async fn open_account(&self, user_id: i64, kind: AccountType, approved: bool) -> Result<Account, AppError>;
    async fn approve_account(&self, account_id: i64) -> Result<(), AppError>;
    // ...
}

pub struct PgAccountService { db: PgPool }   // private field — encapsulated

#[async_trait::async_trait]
impl AccountService for PgAccountService { /* SQLx queries */ }
```

`main.rs` builds each concrete impl once as `Arc<dyn Trait>` and registers it as `web::Data`;
handlers extract by trait (`web::Data<dyn AccountService>`). `PgAdminService` is built last and
the others are injected via `with_services(...)`.

---

## Database conventions

- `snake_case` plural tables; every table has `id BIGSERIAL PRIMARY KEY` and `created_at TIMESTAMPTZ`.
- Foreign keys `<table>_id BIGINT REFERENCES <table>(id)`.
- Money is `NUMERIC(18,2)` ↔ `rust_decimal::Decimal` (never `f64`).
- Queries use **runtime** `sqlx::query` / `sqlx::query_as` (no compile-time macros, so no live DB
  or `.sqlx` cache is needed to build — see the Dockerfile).
- Migrations live in `migrations/` and are applied automatically on startup by both the app and
  the seed binary.

---

## Local development

See **[README.md](./README.md)** for the full Docker and local run instructions. In short:
`docker compose up --build` starts Postgres + app + seed; migrations apply automatically.
