# FerroBank

**Iron-clad core banking, built in Rust.**

A server-side-rendered banking platform for **CSC1106 Web Programming**
(Banking System domain). Concurrency-safe money movement, role-based access,
out-of-band OTPs via Telegram, fraud holds with identity review, and a full
audit trail.


> - Group Number: `G26`
> - Members: `Macarius Dai Chenxuan 2500581`, `Chin Yu Xuan 2502811`, `Heng Ann Ya 2502737`, `Wesley Chandra 2500823`, `Kent Lo Wei Jun 2500343`

---

## Stack

| Layer | Choice |
|---|---|
| Web framework | Actix Web 4 |
| Templates (SSR) | Askama (compile-time checked) |
| Database | PostgreSQL 16 |
| DB access | SQLx |
| Auth | argon2id hashing, cookie sessions, 5-min inactivity TTL |
| OTP / notifications | Telegram Bot API (`reqwest`), on-screen fallback |
| Money | rust_decimal over NUMERIC (never f64) |
| Frontend | Tailwind CSS via CDN, HTMX, vanilla JS toasts |
| Logging | tracing (every request traced with user id) |

## Architecture (summary)

```
Browser ──HTTP──▶ SessionMiddleware ─▶ ActivityGuard ─▶ RequireRole ─▶ Handler (thin)
                                                                          │
                                            Askama template ◀── Service trait (thick)
                                                                          │
                                                          SQLx transactions, FOR UPDATE
                                                                          │
                                                                    PostgreSQL 16
                                                                          ▲
                              Telegram Bot API ◀── OtpChannel / poller ───┘
```

Handlers parse and render; **all business rules live behind service traits**
(`AuthService`, `AccountService`, `TransferService`, `LoanService`,
`ActionOtpService`, `AuditService`, `AdminService`), consumed as
`Arc<dyn Trait>` - encapsulation, abstraction, and runtime polymorphism.
`ActivityGuard` adds a per-user request trail, the 5-minute inactivity TTL
(measured on database time), and mandatory Telegram linking for customers.

Key flows, diagrams, and the ER model: **[docs/FLOWS.md](./docs/FLOWS.md)**,
**[docs/uml_domain_model.mermaid](./docs/uml_domain_model.mermaid)**,
**[docs/uml_service_architecture.mermaid](./docs/uml_service_architecture.mermaid)**,
**[docs/er_diagram.mermaid](./docs/er_diagram.mermaid)**, with a deeper
narrative in **[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)**.

## File layout

```
migrations/        001-007, one per module (users · accounts · transfers ·
                   audit+notifications · loans · repayments · action_otps)
src/
  main.rs          wiring: config, pool, services, middleware, shutdown snapshot
  config.rs        env-driven configuration (DATABASE_URL, TELEGRAM_BOT_TOKEN, …)
  db.rs            Postgres pool
  errors.rs        AppError → HTTP mapping (renders error.html)
  state.rs         AppState (pool, config, bot username)
  view.rs          LayoutCtx, shared OtpConfirmPage
  middleware/      CurrentUser extractor · RequireRole · ActivityGuard
  models/          user · account · transfer · loan (+ enums)
  services/        the seven traits + Pg impls · telegram channel/poller
  handlers/        home · auth · accounts · transfers · loans · settings · admin
  bin/seed.rs      idempotent demo data
templates/         Askama: layout + per-module pages, otp_confirm, review queues
tests/             transfer concurrency/double-spend + shutdown snapshot
docs/              architecture, flows, UML/ER, demo scenarios, runsheet,
                   report outline, proposal, file guide
```

## Running FerroBank

All options serve <http://localhost:8080>. **Docker is optional** - choose A
if you don't want it at all.

### Prerequisites

| Tool | A · Fully local | B · Docker | C · Hybrid |
|---|---|---|---|
| Rust toolchain (rustup) | required | not needed | required |
| PostgreSQL 16 | native install | via Docker | via Docker |
| Docker Desktop | **not needed** | required | required (DB only) |

### Option A - Fully local (no Docker)

```bash
# macOS (Homebrew)
brew install postgresql@16 && brew services start postgresql@16
createuser ferrobank --createdb
createdb ferrobank --owner=ferrobank
psql -d ferrobank -c "ALTER USER ferrobank WITH PASSWORD 'ferrobank';"
```

```powershell
# Windows - installer from postgresql.org, then in "SQL Shell (psql)":
CREATE ROLE ferrobank WITH LOGIN PASSWORD 'ferrobank' CREATEDB;
CREATE DATABASE ferrobank OWNER ferrobank;
```

Then:

```bash
cp .env.example .env     # set SESSION_SECRET (openssl rand -base64 64) + TELEGRAM_BOT_TOKEN
cargo run --bin seed && cargo run
```

### Restarting (Option A)

```bash
# Restart the server only - all data kept (Ctrl-C the old one first).
# Note: restarting logs everyone out (per-boot session keys), sign in again.
cargo run

# Full reset: wipe the database, reseed, and run again in one line.
dropdb ferrobank && createdb ferrobank --owner=ferrobank && cargo run --bin seed && cargo run

# If Postgres itself isn't running (e.g. after a reboot):
brew services restart postgresql@16
```

If `dropdb` complains the database is "being accessed by other users", close
any open psql sessions (`\q`) and stop the server first.

### Option B - Docker (full stack)

```bash
cp .env.example .env
docker compose up --build      # db + app + one-shot idempotent seed
```

Wipe and reseed: `docker compose down -v && docker compose up --build`.

### Option C - Local app + Docker Postgres

```bash
docker compose up -d db
cp .env.example .env
cargo run --bin seed && cargo run
```

## Telegram OTP & notifications

Create a bot with **@BotFather** (`/newbot`), put the token in `.env` as
`TELEGRAM_BOT_TOKEN`, restart. Customers are then **required** to link
Telegram after first sign-in (guide page with a deep link; it refreshes
itself when the bot confirms). Every one-time code - transfers, account
opening, loan applications, profile changes - and every account update
(approvals, holds, declines, freezes) is delivered there; codes never appear
on screen for linked users. Bot commands: `/unlink`, `/help`. Without a
token the app falls back to on-screen demo codes.

## Seeded users

All passwords follow `<name>123`. Customers have NRICs on file; staff have none.

| Email | Role | Notes |
|---|---|---|
| `admin@ferrobank.local` | Admin | dashboard, audit, race demo, reviews |
| `teller@ferrobank.local` | Teller | approvals, reviews, all transfers |
| `alice@ferrobank.local` | Customer | savings $5,000 + checking $2,500 · **limit-change hold = 10 s (demo)** |
| `bob@ferrobank.local` | Customer | savings $1,200 + checking $800 |
| `charlie@ferrobank.local` | Customer | checking $350 · seeded velocity burst |
| `diana@ferrobank.local` | Customer | savings $0 |
| `eve@ferrobank.local` | Customer | savings $7,500 + checking $1,500 |
| `frank@ferrobank.local` | Customer | checking $600 |

Plus seeded loans and historical transfers that exercise the fraud panel
(large $12,500 · a flagged rejection · a velocity burst).

## Security controls (quick reference)

- OTP on **every** sensitive action (argon2-hashed, single-use, 10-min TTL, 3 strikes)
- No self-transfers (service check + DB trigger)
- Per-transfer limits; increases held 12 h (consent popup) before applying
- Fraud holds: >=$10k, structuring (sub-limit transfer emptying a >=$5k
  balance to within $49), >50% drain of a >$5k balance, 4+/1h velocity;
  customer states purpose + NRIC, staff release/deny (insufficient funds is
  pre-checked, never flagged)
- Dual-control balance adjustments: staff edits over $1,000 need approval
  from the other staff role
- Auto-freeze after 3 overdraft attempts in 24 h (suspected hijack)
- 5-minute inactivity TTL on database time; per-user request trail in the log
- Login device tracking: browser + IP per sign-in, first-seen device/network
  alerts the customer, and staff get a per-user activity view
- Risk-based step-up login: a first-seen device or network must also present
  a one-time code before any session is created (known origins stay
  password-only; the very first login is exempt); 3 wrong codes cancel the
  attempt AND block that device/network from signing in for 24 hours
- Every action OTP carries the same 3-strike budget - the third wrong code
  cancels the pending action outright
- Hardened unlink: the bot's /unlink takes effect after 24 h with warnings
  everywhere, cancellable from the website (which an attacker inside a stolen
  Telegram cannot reach); the website's OTP-confirmed unlink stays immediate
  (`/staff/users/{id}`: logins, accounts, transfers)
- Pending loans expire after 7 days; graceful shutdown writes a state snapshot

## Testing

```bash
cargo test                                    # unit tests run anywhere
cargo test --test transfer_concurrency -- --nocapture   # needs live Postgres
cargo test --test shutdown_snapshot -- --nocapture
```

The concurrency tests prove the engine's invariants: no race conditions, no
inconsistent balances, no double spending. The same thing is clickable at
**Admin → Race demo**.

## Documentation

Everything beyond this README lives in [`docs/`](./docs): architecture
narrative, sequence diagrams, UML + ER models, demo scenarios, the
presentation runsheet, the report outline, and the planning proposal.
