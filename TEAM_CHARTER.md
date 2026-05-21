# FerroBank — Team Charter

**Project:** FerroBank, a Rust-native core banking platform
**Stack:** Rust · Actix Web 4 · Askama · SQLx · PostgreSQL · argon2 · Tailwind (CDN) · HTMX
**Team size:** 5
**Architecture:** Layered (Handlers → Services → Models → DB), OOP via traits

---

## Role assignments

Each of the four module owners delivers a **vertical slice** — model, service, handler, templates, and a SQL migration. The Platform Lead owns the integration layer that everyone plugs into and the Admin Dashboard that reads from everyone.

### Member 1 — Platform Lead / Integration Owner *(owner: TBD)*

Owns the skeleton everyone else plugs into. Ships first, then stays one beat ahead.

**Files**

- `Cargo.toml`, `.env.example`, `.gitignore`, `Dockerfile`, `docker-compose.yml`, `README.md`, `ARCHITECTURE.md`
- `src/main.rs`, `src/routes.rs`, `src/config.rs`, `src/db.rs`, `src/state.rs`, `src/errors.rs`
- `src/middleware/auth.rs` — `CurrentUser` extractor, `RequireRole` guard
- `src/handlers/admin.rs`, `src/services/admin_service.rs`, `templates/admin/`
- `templates/layout.html`, `templates/partials/nav.html`, error pages
- `migrations/001_init.sql` (users + roles, just enough for auth middleware to compile)

**Publishes for teammates:** `AppState`, `AppError`, `CurrentUser`, `RequireRole`, `layout.html`.

---

### Member 2 — Auth Module *(owner: TBD)*

**Files**

- `src/models/user.rs` — `User`, `Role` enum (`Customer`, `Teller`, `Admin`), `NewUser`
- `src/services/auth_service.rs` — `AuthService` trait + `PgAuthService` impl: `register`, `login` (argon2 verify), `create_session`
- `src/handlers/auth.rs` — `GET/POST /register`, `GET/POST /login`, `POST /logout`
- `templates/auth/login.html`, `templates/auth/register.html`
- `migrations/002_users.sql` (extends 001 if needed)
- Form validation via the `validator` crate

**Exposes:** `pub fn routes(cfg: &mut web::ServiceConfig)` from `handlers/auth.rs`.

---

### Member 3 — Accounts Module *(owner: TBD)*

**Files**

- `src/models/account.rs` — `Account`, `AccountType` (`Savings`, `Checking`), `AccountStatus` (`Active`, `Frozen`, `Closed`)
- `src/services/account_service.rs` — `AccountService` trait + `PgAccountService`: `open_account`, `close_account`, `freeze_account`, `get_balance`, `list_for_user`
- `src/handlers/accounts.rs` — `/accounts`, `/accounts/new`, `/accounts/:id`
- `templates/accounts/list.html`, `detail.html`, `new.html`
- `migrations/003_accounts.sql`

**Exposes for Admin Dashboard:** `count_active() -> Result<i64>`, `total_deposits() -> Result<Decimal>`.

---

### Member 4 — Transfers Module *(owner: TBD — assign your strongest Rust dev)*

The headline feature for the demo and the report.

**Files**

- `src/models/transfer.rs` — `Transfer`, `TransferStatus`
- `src/services/transfer_service.rs` — `TransferService` trait + `PgTransferService`
- `src/services/audit_service.rs` — immutable append-only audit log
- `src/handlers/transfers.rs` — `/transfers/new`, `/transfers/confirm`, `/transfers/history`
- `templates/transfers/new.html`, `confirm.html`, `history.html`
- `migrations/004_transfers.sql`, `migrations/005_audit_log.sql`

**Correctness requirements (non-negotiable):**

1. Every transfer runs inside a single `BEGIN ... COMMIT` SQL transaction.
2. Lock both rows with `SELECT ... FOR UPDATE` before reading balances.
3. Reject on: insufficient funds, frozen/closed accounts, self-transfer, negative amounts.
4. Write one audit log row per attempt (success or failure).
5. OTP simulation: generate 6-digit code, store hashed, verify on the `/confirm` step.

**Demo moment:** an integration test that fires 50 concurrent transfers between the same two accounts and asserts the final sum is unchanged. Put this in `tests/concurrent_transfers.rs`.

**Exposes for Admin Dashboard:** `recent(limit) -> Result<Vec<Transfer>>`, `flagged() -> Result<Vec<Transfer>>`.

---

### Member 5 — Loans Module *(owner: TBD)*

**Files**

- `src/models/loan.rs` — `Loan`, `LoanStatus`, `Repayment`
- `src/services/loan_service.rs` — `LoanService` trait + `PgLoanService`: apply, approve (admin-only), reject, amortization schedule, record repayment, outstanding balance
- `src/handlers/loans.rs` — `/loans/apply`, `/loans`, `/loans/:id`, `/loans/:id/repay`
- `templates/loans/apply.html`, `list.html`, `detail.html`, `repay.html`
- `migrations/006_loans.sql`, `migrations/007_repayments.sql`

**Money math:** always `rust_decimal::Decimal`, stored as `NUMERIC(18,2)` in Postgres. Never `f64`.

**Exposes for Admin Dashboard:** `pending_applications() -> Result<Vec<Loan>>`, `portfolio_outstanding() -> Result<Decimal>`.

---

## The shared contract (lock this on day 1)

These conventions are what let four people work in parallel without colliding. Everyone must follow them — code review will reject violations.

1. **Routes.** Every module exports `pub fn routes(cfg: &mut web::ServiceConfig)` from its top handler file. Only `src/routes.rs` calls these. Nobody else mounts routes.
2. **Services as traits.** Every module defines `pub trait XxxService: Send + Sync` with at least one concrete `PgXxxService` impl. Handlers depend on the trait via `web::Data<dyn XxxService>`, not the concrete type. This is the OOP/polymorphism evidence for the report.
3. **Errors.** Every service returns `Result<T, crate::errors::AppError>`. No `.unwrap()`, no `.expect()`, no `panic!()` in handler or service code. The `?` operator is your friend.
4. **Templates.** Every page starts with `{% extends "layout.html" %}`. Module templates live in `templates/<module>/`. Use Askama, compile-time checked.
5. **Auth.** Any handler that needs the logged-in user takes `user: CurrentUser` as an extractor. Admin-only routes wrap with `RequireRole(Role::Admin)`.
6. **Migrations.** Sequentially numbered. Before writing one, announce the number in the team chat so two people don't pick the same one. Platform Lead resolves conflicts on merge.
7. **Money.** Always `rust_decimal::Decimal`. Stored as `NUMERIC(18,2)`. Never `f64`. The grader will be looking for this.
8. **Branching.** One feature branch per task, named `<module>/<short-desc>` (e.g., `transfers/concurrent-lock`). PR into `main` with at least one teammate's review.
9. **Formatting.** Run `cargo fmt` before every commit. CI will fail otherwise.
10. **No secrets in git.** Use `.env` (gitignored). `.env.example` is the template.

---

## Sequencing

### Week 1 — Skeleton & migrations

- **Platform Lead** ships the skeleton: project compiles, boots, `/` renders "Hello FerroBank" using `layout.html`, Postgres connects, auth middleware shell in place.
- **Module owners** each write their migration and `models/xxx.rs` struct. DB schema compiles end-to-end at the end of the week.
- Daily 15-min standup. Lock the shared contract above.

### Week 2 — Vertical slices

- Each module owner implements service trait + handlers + templates against the skeleton.
- Platform Lead builds the Admin Dashboard incrementally as other modules' read methods come online.

### Week 3 — Integration, polish, demo

- Transfers concurrent-test must pass.
- Screenshots, README polish, presentation slides, demo recording.

---

## Definition of done (per module)

A module is "done" when:

- [ ] Migration applies cleanly on a fresh DB.
- [ ] All handlers return correct HTTP status codes (200, 302 redirect, 4xx for validation, never 500 on user input).
- [ ] Templates extend `layout.html` and render without errors.
- [ ] Service has at least two unit tests (happy path + one failure path).
- [ ] Form inputs validated via the `validator` crate.
- [ ] No `unwrap`, no `expect`, no `panic!` in non-test code.
- [ ] `cargo fmt` and `cargo clippy` both pass clean.

---

## Things to NOT do

- Don't call into another module's database tables directly. Always go through the other module's service trait.
- Don't add new top-level dependencies without a discussion. The platform lead curates `Cargo.toml`.
- Don't refactor someone else's module without telling them first.
- Don't store passwords as anything other than argon2 hashes.
- Don't use `f64` for money. Ever.
