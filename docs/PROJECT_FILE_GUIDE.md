# FerroBank — File-by-File Guide

A complete tour of every file in the project and what it does. The codebase follows
a **layered architecture**: a request flows

```
HTTP → handler (routes/parse) → service (business logic, trait) → model/DB (sqlx) → Askama template → HTML
```

Handlers depend on service **traits** (`web::Data<dyn XxxService>`), never concrete
types — that's the OOP/polymorphism story and what makes modules swappable/testable.

---

## Root — configuration & meta

| File | Purpose |
|---|---|
| `Cargo.toml` | Crate manifest: dependencies (Actix Web, SQLx, Askama, argon2, rust_decimal, tokio, etc.) and the two binaries (`ferrobank`, `seed`). |
| `Cargo.lock` | Exact resolved dependency versions for reproducible builds. |
| `Dockerfile` | Two-stage build: compile the release binaries in a Rust image, then copy them into a slim Debian runtime image. |
| `docker-compose.yml` | Orchestrates three services: `db` (Postgres 16), `app` (the server), and `seed` (one-shot demo-data loader that runs on `up`). |
| `.env` | Your local environment values (DB URL, host/port, `SESSION_SECRET`). Not committed. |
| `.env.example` | Template for `.env` so teammates know which variables to set. |
| `.gitignore` | Keeps `target/`, `.env`, etc. out of version control. |
| `README.md` | Project overview + how to run (Docker and local), seeded users, dev workflow. |
| `ARCHITECTURE.md` | The layered design, module boundaries, and the contract each module follows. |
| `TEAM_CHARTER.md` | Who owns which module (the group/individual split) and coordination notes. |
| `DEMO_SCENARIOS.md` | Step-by-step demonstration script mapped to the marking criteria. |
| `REPORT_OUTLINE.md` | Suggested structure for the project report. |
| `PRESENTATION_SCRIPT.md` | Slide deck + live-demo runsheet for the recording (team of 5). |
| `docs/uml_domain_model.mermaid` | UML class diagram of the domain entities + enums. |
| `docs/uml_service_architecture.mermaid` | UML class diagram of the service traits, impls, and HTTP layer. |
| `PROJECT_FILE_GUIDE.md` | This document. |
| `LICENSE` | MIT license. |
| `static/.gitkeep` | Placeholder so the (otherwise empty) `static/` directory — served at `/static` — is tracked by git. |

---

## `migrations/` — database schema (SQLx, applied in order)

These run automatically on startup (by both the app and the seed binary). The schema
is fully normalised; every table links back to `users`/`accounts` via foreign keys.

| File | Creates |
|---|---|
| `001_init.sql` | `user_role` enum (`customer`/`teller`/`admin`) and the `users` table. |
| `002_accounts.sql` | `account_type` + `account_status` enums (status includes **`pending`** for teller approval) and the `accounts` table, with a non-negative-balance check **and a trigger enforcing that only customers may own accounts**. |
| `003_transfers.sql` | `transfer_status` enum and the `transfers` table — includes `otp_hash`, `status_reason` (rejection/flag explanation), and positive-amount / no-self-transfer checks. |
| `004_audit_log.sql` | `audit_log` table — append-only event log with a JSONB payload. |
| `005_loans.sql` | `loan_status` enum, the `loans` table, **and** the `loan_approvals` table that powers dual (teller+admin) approval. |
| `006_repayments.sql` | `repayments` table — includes `account_id`, the funding account each repayment is debited from. |

> Note: the former `007_feature_upgrades.sql` has been folded into 003/005/006 so the
> schema reads as one coherent design. Because that changes existing migration
> checksums, reset the database when upgrading: `docker compose down -v` then `up --build`.

---

## `src/` — application entry points

| File | Purpose |
|---|---|
| `main.rs` | Binary entrypoint: loads config, connects to Postgres, runs migrations, builds the Actix `App`, wires every service into shared state, configures the session middleware, and serves HTTP. |
| `lib.rs` | The library crate root — declares all modules (`config`, `db`, `errors`, `handlers`, `middleware`, `models`, `routes`, `services`, `state`, `view`). `main.rs` and `seed.rs` both build on this. |
| `bin/seed.rs` | Standalone `seed` binary. Applies migrations, then inserts demo users, accounts, loans (with teller+admin approvals), backdated transfers, and audit entries. Idempotent. |

---

## `src/` — platform infrastructure (Member 1, Platform Lead)

| File | Purpose |
|---|---|
| `config.rs` | `Config::from_env()` — reads `DATABASE_URL`, `APP_HOST`, `APP_PORT`, `SESSION_SECRET` (and validates the secret is ≥ 64 bytes). |
| `db.rs` | `connect()` — builds the `PgPool` (connection pool) with sensible limits/timeouts. |
| `state.rs` | `AppState { db, config }` — shared state injected into handlers via `web::Data<AppState>`. |
| `errors.rs` | `AppError` — the single error type every service/handler returns. Implements Actix's `ResponseError` so `?` maps cleanly to HTTP status codes and a rendered error page (and bounces `Unauthorized` to `/login`). |
| `view.rs` | `LayoutCtx` / `UserChip` — the view-model the base layout reads to render the nav (who's logged in, their role). `from_user()` / `anonymous()`. |
| `routes.rs` | The single place that mounts every module's routes onto the Actix app. |

---

## `src/middleware/` — auth guard & extractors

| File | Purpose |
|---|---|
| `mod.rs` | Re-exports `CurrentUser`, `RequireRole`, `SessionUser`. |
| `auth.rs` | `CurrentUser` request extractor (drop into a handler to require login; `Option<CurrentUser>` for "maybe logged in"); `RequireRole(role)` middleware to protect a whole scope; and `session::login/logout` helpers. |

---

## `src/models/` — domain entities (one file per owner)

These are the Rust structs that map to database rows (`#[derive(FromRow)]`) plus
small display helpers. Money is always `rust_decimal::Decimal`, never `f64`.

| File | Purpose |
|---|---|
| `mod.rs` | Declares the model modules. |
| `user.rs` | `User`, `NewUser`, and the `Role` enum (`label()`). |
| `account.rs` | `Account` plus `AccountType` and `AccountStatus` enums, with `label()` and `badge()` (Tailwind classes for status pills). |
| `transfer.rs` | `Transfer` plus `TransferStatus` enum (`label()`, `badge()`). Carries `status_reason`. |
| `loan.rs` | `Loan` (with `rate_pct()` → e.g. `"5.25%"`), `LoanStatus` enum, and `Repayment` (with its funding `account_id`). |

---

## `src/services/` — business logic (trait + Postgres impl)

The heart of the app. Each service is a trait (the abstraction handlers depend on)
with a `Pg…` implementation. This is where transactions, locks, and rules live.

| File | Purpose |
|---|---|
| `mod.rs` | Declares the service modules and explains the trait-based design. |
| `auth_service.rs` | Register/login/lookup users; argon2id password hashing & verification. |
| `account_service.rs` | Open accounts (pending or active), **approve a pending account**, close/freeze/**unfreeze**, **adjust balance** (admin), balances, and dashboard counts. |
| `transfer_service.rs` | The concurrency-safe transfer engine: in-memory `Mutex` rate limiter, OTP generation/verification, and the two-step create→confirm flow that moves money under `SELECT … FOR UPDATE` row locks inside one transaction, writing rejection reasons and audit events. |
| `audit_service.rs` | Append-only `record()` of events + `recent()` retrieval for the audit log. |
| `loan_service.rs` | Apply for loans, **dual approval** (one teller + one admin, enforced distinct via `loan_approvals`), reject, **repayment that debits a funding account**, outstanding-balance maths (rounded to 2 dp), and portfolio totals. |
| `admin_service.rs` | Composes a dashboard snapshot from the other services, and provides `all_accounts` / `all_transfers` / `all_users` (joined to owners) for the admin management screens. |

---

## `src/handlers/` — Actix routes (one file per module)

Thin layer: parse the request, call a service, render a template (or redirect).
Each exposes `pub fn routes(cfg)`; `routes.rs` mounts them.

| File | Routes / purpose |
|---|---|
| `mod.rs` | Declares the handler modules. |
| `home.rs` | `GET /` — the public landing page (session-aware, so the nav/CTAs reflect login state). |
| `auth.rs` | `GET/POST /login`, `GET/POST /register`, `POST /logout`. Registered as plain routes (no empty scope — that earlier caused 404s). |
| `accounts.rs` | Customer account pages: list, open, detail, freeze, close. |
| `transfers.rs` | Transfer history, new-transfer form (from-account dropdown), create (resolves recipient name for the confirm page), and OTP confirm. |
| `loans.rs` | Loan list (customer view vs staff review queue), apply, detail (with approval progress + repay form), repay (account selection), approve (teller/admin), reject. |
| `admin.rs` | Two scopes. `/admin` (`RequireRole(Admin)`): dashboard with fraud signals, searchable audit log, account mutations (open/freeze/unfreeze/close/adjust). `/staff` (`RequireRole(Teller)` — teller **and** admin): all-accounts page with **account approval**, and all-transfers page with a date-range filter. |

---

## `templates/` — Askama server-side rendering

Compile-time-checked HTML templates. Tailwind (via CDN) for styling; a shared base
layout provides the nav, footer, light/dark theme, and the table-search helper.

**Shared chrome**

| File | Purpose |
|---|---|
| `layout.html` | Base layout every page extends: `<head>`, theme system (light/dark, pre-paint script), transitions, the footer, and shared JS (`fbToggleTheme`, `fbFilterTable`). |
| `partials/nav.html` | Top navigation bar — role-aware (hides customer Accounts/Transfers tabs for admins), theme toggle, sign-in/out. |
| `home.html` | Landing page hero + feature cards + trust section; CTAs change when logged in. |
| `error.html` | Friendly error page rendered by `AppError`. |

**Auth**

| File | Purpose |
|---|---|
| `auth/login.html` | Sign-in form. |
| `auth/register.html` | Registration form. |

**Accounts**

| File | Purpose |
|---|---|
| `accounts/list.html` | Customer's accounts + total balance. |
| `accounts/new.html` | Choose savings/checking to open. |
| `accounts/detail.html` | One account's detail + manage actions. |

**Transfers**

| File | Purpose |
|---|---|
| `transfers/history.html` | In/out transfer history with status and rejection reasons. |
| `transfers/new.html` | New-transfer form (from-account dropdown, recipient number, amount, note). |
| `transfers/confirm.html` | Step 2: shows resolved recipient name + the demo OTP, takes the confirmation code. |

**Loans**

| File | Purpose |
|---|---|
| `loans/list.html` | Customer's loans, or the staff pending-review queue (rates shown as %). |
| `loans/apply.html` | Loan application form. |
| `loans/detail.html` | Loan summary, dual-approval progress + staff approve/reject, and the repay form with funding-account dropdown. |

**Admin**

| File | Purpose |
|---|---|
| `admin/dashboard.html` | KPI cards, recent transfers, flagged transfers (with reasons), pending loan queue, and links to the management pages. |
| `admin/accounts.html` | All accounts with search + CRUD (open for a user, freeze/unfreeze, close, adjust balance). |
| `admin/transfers.html` | All transfers with search and a reason/flag column. |
| `admin/audit.html` | Searchable audit log (latest 500 events). |

---

## Added during the hardening pass (June 2026)

| File | What it is |
|---|---|
| `migrations/007_telegram.sql` | Telegram linking columns on `users` (chat id, single-use link code, phone) |
| `migrations/008_names_and_action_otps.sql` | First/middle/last names (+backfill), the `action_otps` table, and the no-self-transfer trigger |
| `src/services/telegram_service.rs` | `OtpChannel` trait (`TelegramOtp` / `ScreenOtp`), `getMe` helper, `getUpdates` poller with `/start`, `/unlink`, `/help` commands |
| `src/services/action_otp_service.rs` | Generalized OTP guard: park a sensitive action, verify a single-use expiring code, return the payload |
| `src/handlers/settings.rs` + `templates/settings/telegram.html` | Telegram linking guide page + OTP-gated unlink |
| `templates/otp_confirm.html` | Shared confirmation page for all OTP-gated actions (`OtpConfirmPage` in `src/view.rs`) |
| `templates/admin/race_demo.html` (+ handlers in `admin.rs`) | The concurrency lab: fire N simultaneous transfers, watch invariants hold |
| `tests/transfer_concurrency.rs` | Race-condition + double-spend integration tests (DB-gated) |
| `tests/shutdown_snapshot.rs` | Verifies the graceful-shutdown `system.snapshot` audit row |
| `docs/FLOWS.md` | Mermaid sequence diagrams for every workflow |
| `PROPOSAL.md` | Formative-assessment-style proposal draft (rewrite before submitting) |


## Security-hardening pass additions

| File | What it is |
|---|---|
| `migrations/001–007` (consolidated) | One clean migration per module — names/NRIC/Telegram on users, transfer limits + `limit_changes`, `on_hold` transfers + `transfer_reviews`, `notifications`, loan disbursement, `action_otps` |
| `src/middleware/auth.rs::ActivityGuard` | Global middleware: per-user request log, 5-min inactivity TTL on DB time, mandatory Telegram linking for customers |
| `src/handlers/transfers.rs` (review) + `templates/transfers/review.html` | Customer side of the fraud-hold pipeline (purpose + NRIC) |
| `src/handlers/admin.rs` (review queue) + `templates/admin/review.html` | Staff release/deny queue with NRIC comparison |
| `templates/otp_confirm.html` + `src/services/action_otp_service.rs` | One OTP page + service guarding every sensitive action |
| `docs/er_diagram.mermaid` | Entity-relationship diagram of the full schema |
