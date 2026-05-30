# FerroBank — Team Charter

**Project:** FerroBank, a Rust-native core banking platform (CSC1106 Web Programming, v1.2 spec)
**Domain:** Banking System
**Stack:** Rust · Actix Web 4 · Askama · SQLx · PostgreSQL · argon2 · Tailwind (CDN) · HTMX
**Team size:** 5
**Architecture:** Layered (Handlers → Services → Models → DB), OOP via traits

> **Group submission identifying info must appear on every deliverable** (source zip, recording, slides, report):
> Group Number · Student Name(s) · Student ID(s) (SIT).
> Add a `GROUP_INFO.md` once the group number is known.

---

## Role assignments

Each module owner delivers a **vertical slice** — model, service trait + impl, handler, templates, and a SQL migration. The Platform Lead owns the integration layer that everyone plugs into and the Admin Dashboard that reads from everyone.

Each member also owns one **Individual Extended Feature** that demonstrates technical depth beyond the group baseline. This is the 40% individual portion of the grade.

---

### Member 1 — Platform Lead / Integration Owner & Admin Dashboard  *(owner: TBD)*

Owns the skeleton everyone else plugs into. Ships first, then stays one beat ahead.

**Group baseline**

- `Cargo.toml`, `.env.example`, `.gitignore`, `Dockerfile`, `docker-compose.yml`, `README.md`, `ARCHITECTURE.md`
- `src/main.rs`, `src/routes.rs`, `src/config.rs`, `src/db.rs`, `src/state.rs`, `src/errors.rs`
- `src/middleware/auth.rs` — `CurrentUser` extractor, `RequireRole` guard
- `src/handlers/admin.rs`, `src/services/admin_service.rs`, `templates/admin/`
- `templates/layout.html`, `templates/partials/nav.html`, `templates/error.html`
- `migrations/001_init.sql` (users + roles)

**Publishes for teammates:** `AppState`, `AppError`, `CurrentUser`, `RequireRole`, `LayoutCtx`, `layout.html`.

**Individual Extended Feature:** **Cross-module analytics dashboard with audit-log explorer.**
A read-only admin view that joins data from every module — KPI cards, recent transfer table, flagged transfers, pending loan applications, full audit log timeline. Demonstrates composing multiple service traits, role-based access via middleware, and view-model aggregation.

---

### Member 2 — Auth Module  *(owner: TBD)*

**Group baseline**

- `src/models/user.rs` — `User`, `Role` enum (`Customer`, `Teller`, `Admin`), `NewUser`
- `src/services/auth_service.rs` — `AuthService` trait + `PgAuthService` impl: `register`, `login` (argon2), `find_by_id`
- `src/handlers/auth.rs` — `GET/POST /register`, `GET/POST /login`, `POST /logout`
- `templates/auth/login.html`, `templates/auth/register.html`
- Form validation via the `validator` crate

**Exposes:** `pub fn routes(cfg: &mut web::ServiceConfig)` from `handlers/auth.rs`.

**Individual Extended Feature:** **Advanced authentication — account lockout & two-step OTP login.**
After 5 failed login attempts within 15 minutes, lock the account for 15 minutes (in-memory `Arc<RwLock<HashMap<…>>>` cache plus durable record in the `auth_attempts` table). Optional second step: email-OTP simulation that logs the OTP to the tracing console for staff demo. Demonstrates Rust concurrency primitives, time-window rate limiting, and secure auth design.

---

### Member 3 — Accounts Module  *(owner: TBD)*

**Group baseline**

- `src/models/account.rs` — `Account`, `AccountType` (`Savings`, `Checking`), `AccountStatus` (`Active`, `Frozen`, `Closed`)
- `src/services/account_service.rs` — `AccountService` trait + `PgAccountService` impl: `open_account`, `close_account`, `freeze_account`, `get_balance`, `list_for_user`
- `src/handlers/accounts.rs` — `/accounts`, `/accounts/new`, `/accounts/:id`
- `templates/accounts/list.html`, `detail.html`, `new.html`
- `migrations/002_accounts.sql`

**Exposes for Admin Dashboard:** `count_active() -> Result<i64>`, `total_deposits() -> Result<Decimal>`.

**Individual Extended Feature:** **Monthly account statement PDF export.**
Generate a downloadable PDF statement for any account showing opening balance, every credit/debit for the month, running balance, and closing balance. Use the `printpdf` or `genpdf` crate. Demonstrates PDF generation, date-range queries, and money-formatting precision.

---

### Member 4 — Transfers Module  *(owner: TBD — assign your strongest Rust developer)*

The headline feature for the demo and the report.

**Group baseline**

- `src/models/transfer.rs` — `Transfer`, `TransferStatus`
- `src/services/transfer_service.rs` — `TransferService` trait + `PgTransferService`
- `src/services/audit_service.rs` — immutable append-only audit log
- `src/handlers/transfers.rs` — `/transfers/new`, `/transfers/confirm`, `/transfers/history`
- `templates/transfers/new.html`, `confirm.html`, `history.html`
- `migrations/004_transfers.sql`, `migrations/005_audit_log.sql`

**Correctness requirements (non-negotiable):**

1. Every transfer runs inside a single `BEGIN ... COMMIT` SQL transaction.
2. Lock both rows with `SELECT ... FOR UPDATE` before reading balances; **order locks by account id** to avoid deadlocks.
3. Reject on: insufficient funds, frozen/closed accounts, self-transfer, negative amounts.
4. Write one audit log row per attempt (success OR rejection).
5. OTP simulation: generate a 6-digit code, store hashed, verify on the `/confirm` step.
6. **Application-level concurrency primitive (v1.2 spec requirement):** in addition to the DB row lock, maintain an `Arc<tokio::sync::Mutex<HashMap<i64, ()>>>` of per-account in-flight transfer guards, OR an `Arc<RwLock<…>>` OTP cache. The report must explain why both layers exist and what each protects against.
7. **Fraud detection rules (v1.2 spec advanced feature):** rules engine that flags any of: amount > $10,000 single transfer; > 5 transfers in 10 minutes from one account; transfer to an account < 24 hours old; transfer between accounts owned by the same user above $5,000.

**Demo moment:** an integration test that fires 50 concurrent transfers between the same two accounts and asserts the final sum is unchanged. Put this in `tests/concurrent_transfers.rs`.

**Exposes for Admin Dashboard:** `recent(limit) -> Result<Vec<Transfer>>`, `flagged() -> Result<Vec<Transfer>>`.

**Individual Extended Feature:** **Concurrency-safe transfer engine + fraud rules engine + audit log.**
The non-negotiables above ARE the individual extended feature for Member 4. Document each design decision in the report: lock ordering, why both Mutex and FOR UPDATE, rollback semantics, fraud rule definitions, audit immutability.

---

### Member 5 — Loans & Fixed Deposits Module  *(owner: TBD)*

> v1.2 spec lists **Fixed Deposit Systems** as a Banking module. It pairs naturally with Loans — both are time-based money products with interest accrual and scheduled cash flows.

**Group baseline**

- `src/models/loan.rs` — `Loan`, `LoanStatus`, `Repayment`
- `src/models/fixed_deposit.rs` — `FixedDeposit`, `FixedDepositStatus`
- `src/services/loan_service.rs` — `LoanService` trait + `PgLoanService`: apply, approve (admin-only), reject, amortization schedule, record repayment, outstanding balance
- `src/services/fixed_deposit_service.rs` — `FixedDepositService` trait + `PgFixedDepositService`: open, calculate maturity value, early withdrawal with penalty, mature (credit back to linked account)
- `src/handlers/loans.rs` — `/loans/apply`, `/loans`, `/loans/:id`, `/loans/:id/repay`
- `src/handlers/fixed_deposits.rs` — `/deposits`, `/deposits/new`, `/deposits/:id`, `/deposits/:id/withdraw`
- `templates/loans/*.html`, `templates/deposits/*.html`
- `migrations/006_loans.sql`, `migrations/007_repayments.sql`, `migrations/008_fixed_deposits.sql`

**Money math:** always `rust_decimal::Decimal`, stored as `NUMERIC(18,2)` in Postgres. Never `f64`.

**Exposes for Admin Dashboard:** `pending_applications() -> Result<Vec<Loan>>`, `portfolio_outstanding() -> Result<Decimal>`, `total_fd_value() -> Result<Decimal>`.

**Individual Extended Feature:** **Amortization scheduler + interest-accrual engine.**
For loans: compute and persist the full amortization schedule on approval (principal + interest split per month). For fixed deposits: compute daily-compounded interest accrual, project maturity value, and apply early-withdrawal penalty correctly. Demonstrates decimal-precise financial math and scheduled cash-flow modelling.

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
8. **Concurrency.** Where appropriate, prefer `tokio::sync::Mutex` / `tokio::sync::RwLock` over `std::sync` for async code paths. Document every concurrency primitive in the report.
9. **Branching.** One feature branch per task, named `<module>/<short-desc>` (e.g., `transfers/concurrent-lock`). PR into `main` with at least one teammate's review.
10. **Formatting.** Run `cargo fmt` before every commit. CI will fail otherwise.
11. **No secrets in git.** Use `.env` (gitignored). `.env.example` is the template.

---

## Sequencing

### Week 1 — Skeleton & migrations
- Platform Lead ships the skeleton: project compiles, boots, `/` renders, Postgres connects, auth middleware in place.
- Module owners each write their migration and `models/xxx.rs` struct. DB schema compiles end-to-end at the end of the week.
- Daily 15-min standup. Lock the shared contract above.
- **Begin literature review for the report** — Cyclos, Mambu, any open-source core banking project.

### Week 2 — Vertical slices
- Each module owner implements service trait + handlers + templates.
- Platform Lead builds the Admin Dashboard incrementally as other modules' read methods come online.

### Week 3 — Individual extended features
- Each member focuses on their named extended feature.
- Member 4's concurrent-transfer test must pass.

### Week 4 — Demo, report, polish
- Demo recording (15 min, ≤200 MB, `g##_recording.mp4`).
- Slides (`.pptx` + `.pdf`, ≤20 MB each).
- Report (≤6 pages, `.docx` + `.pdf`, ≤20 MB each).
- Final source zip (`g##_source.zip`, ≤20 MB).

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
- [ ] Individual extended feature implemented and documented in the report.

---

## Things to NOT do

- Don't call into another module's database tables directly. Always go through the other module's service trait.
- Don't add new top-level dependencies without a discussion. The platform lead curates `Cargo.toml`.
- Don't refactor someone else's module without telling them first.
- Don't store passwords as anything other than argon2 hashes.
- Don't use `f64` for money. Ever.
- Don't catch generic panics — let the global error handler render the friendly error page.

---

## Submission deliverables checklist

The group leader uploads each of the following to xSiTe Dropbox separately. Every file must include Group Number, Student Name(s), and Student ID(s) somewhere visible.

| # | File | Format | Max size |
|---|---|---|---|
| 1 | Source code archive | `g##_source.zip` | 20 MB |
| 2 | Demo recording (15 min) | `g##_recording.mp4` | 200 MB |
| 3 | Presentation slides | `g##_slides.pptx` **and** `g##_slides.pdf` | 20 MB each |
| 4 | Project report (≤6 pages) | `g##_report.docx` **and** `g##_report.pdf` | 20 MB each |

The report must include, for every member: their **group contribution** and their **individual extended feature** with justification and a brief literature reference where relevant.
