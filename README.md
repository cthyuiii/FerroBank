# FerroBank

**Iron-clad core banking, built in Rust.**

A server-side-rendered enterprise banking platform built for **CSC1106 Web Programming** at SIT
(spec v1.2 — Banking System domain).
Demonstrates layered architecture, OOP via traits, ACID money movement with both database-level
and application-level concurrency control, role-based access control, fraud detection, and audit logging.

> **Group / Author info** — to be filled in by the group leader before submission:
>
> - Group Number: `g##`
> - Members: `Name 1 (SIT ID)`, `Name 2 (SIT ID)`, `Name 3 (SIT ID)`, `Name 4 (SIT ID)`, `Name 5 (SIT ID)`

---

## Stack

| Layer | Choice |
|---|---|
| Web framework | Actix Web 4 |
| Templates (SSR) | Askama (compile-time checked) |
| Database | PostgreSQL 16 |
| DB access | SQLx (compile-time checked SQL) |
| Auth | argon2 password hashing, actix-session cookies |
| Money | rust_decimal (never f64) |
| Frontend | Tailwind CSS via CDN, HTMX for partial updates |
| Logging | tracing + tracing-subscriber |

See **[ARCHITECTURE.md](./ARCHITECTURE.md)** for how the pieces fit and **[TEAM_CHARTER.md](./TEAM_CHARTER.md)** for who owns what.

---

## Modules

| Module | Owner | Description |
|---|---|---|
| Auth | M2 | Registration, login, argon2 password hashing, cookie sessions, roles (customer/teller/admin) |
| Accounts | M3 | Open/close/freeze accounts, balances, savings & checking, **teller-approved opening** (pending → active), admin balance adjustments |
| Transfers | M4 | ACID money movement, **Mutex rate-limit + row-lock concurrency + explicit rollback**, OTP confirm, rejection reasons, fraud rules, audit log |
| Loans | M5 | Applications, simple-interest model, **dual approval (teller + admin)**, repayments that debit a funding account |
| Admin & Staff | M1 (Platform Lead) | Admin dashboard with fraud signals, searchable audit log, all-accounts CRUD; staff (teller) area for account approval and transfer review |

See **[TEAM_CHARTER.md](./TEAM_CHARTER.md)** for each member's group baseline and their individual extended feature (which together cover the 60% group + 40% individual marking criteria).

## Roles & access

| Role | Lands on | Can do |
|---|---|---|
| Customer | `/accounts` | Open accounts (pending approval), transfer money (OTP), apply for and repay loans |
| Teller | `/loans` | Review **all** loans + record the teller approval; **approve accounts** and view all transfers under `/staff/*` |
| Admin | `/admin/dashboard` | Everything: dashboard + fraud signals, audit log, full account CRUD, the admin loan approval |

> Banking-domain highlights for the spec: a concurrency-safe transfer engine (Mutex + `SELECT … FOR UPDATE` + rollback), OTP simulation, audit logging, fraud detection (large / structuring / velocity / rejected), and dual-control approvals. See **[docs/uml_domain_model.mermaid](./docs/uml_domain_model.mermaid)** and **[docs/uml_service_architecture.mermaid](./docs/uml_service_architecture.mermaid)** for the class diagrams.

---

## Running FerroBank

There are two supported ways to run the project. Both serve the app at
<http://localhost:8080>.

- **Docker (recommended)** — one command builds and starts Postgres, the app,
  and the demo data. No Rust toolchain required.
- **Local (`cargo`)** — run the app directly with Cargo against a Postgres
  container. Best for active Rust development and fast rebuilds.

### Prerequisites

| Tool | Docker run | Local run |
|---|---|---|
| Docker Desktop | required | required (for Postgres) |
| Rust toolchain (rustup) | not needed | required |
| SQLx CLI | not needed | optional (only to author new migrations) |

One-time installs:

```bash
# macOS
brew install --cask docker          # then: open -a Docker
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust (local run only)
```

```powershell
# Windows (PowerShell) — restart the shell afterwards so PATH updates
winget install Docker.DockerDesktop  # launch it once from the Start Menu
winget install Rustlang.Rustup       # Rust (local run only)
```

> No `winget`? Grab installers from <https://www.docker.com/products/docker-desktop/>
> and <https://rustup.rs/>. On Windows, Git Bash lets you follow the macOS
> commands (`cp`, `openssl`, `source`) verbatim.

---

### Option A — Docker (full stack)

```bash
cp .env.example .env          # optional, but recommended: set your own SESSION_SECRET
docker compose up --build
```

That builds and starts three services:

| Service | What it does |
|---|---|
| `db` | PostgreSQL 16 (data persisted in the `ferrobank_pg` volume) |
| `app` | The FerroBank server — applies migrations on startup, serves on `:8080` |
| `seed` | One-shot job: applies migrations, inserts demo data, then **exits** (showing as "exited" is expected) |

Common variations:

```bash
docker compose up --build -d                            # run in the background
docker compose logs -f app                              # tail the app logs
docker compose down                                     # stop everything (keeps data)
docker compose down -v && docker compose up --build     # wipe all data and reseed from scratch
```

The seed is idempotent, so it's safe on every `up`. To run a **clean instance with
no demo data** (e.g. for a real deployment), re-add `profiles: ["seed"]` to the
`seed` service in `docker-compose.yml`; it will then only run when you ask for it
explicitly with `docker compose run --rm seed`.

---

### Option B — Local (`cargo`)

Postgres still runs in Docker; only the app runs natively.

```bash
# 1. Start just Postgres
docker compose up -d db

# 2. Configure env
cp .env.example .env

# 3. Generate a 64+ byte session secret and paste it into .env as SESSION_SECRET=...
openssl rand -base64 64

# 4. Seed demo data (this also applies migrations)
cargo run --bin seed

# 5. Run the app (also applies any pending migrations on startup)
cargo run
```

Windows PowerShell equivalent for steps 2–3:

```powershell
Copy-Item .env.example .env
$bytes = New-Object byte[] 64
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
[Convert]::ToBase64String($bytes)
```

> **No SQLx CLI needed to get started.** Both the app and the seed binary run the
> migrations automatically on startup, and the Postgres container already creates
> the `ferrobank` database — so `sqlx database create` / `sqlx migrate run` are not
> required just to boot. Install `sqlx-cli` only when you want to *author* new
> migrations (see Development workflow).

---

### Default seeded users

Created by the `seed` job (Docker) or `cargo run --bin seed` (local):

| Email | Password | Role |
|---|---|---|
| `admin@ferrobank.local`   | `admin123` *(change me)* | Admin |
| `teller@ferrobank.local`  | `teller123`  | Teller |
| `alice@ferrobank.local`   | `alice123`   | Customer |
| `bob@ferrobank.local`     | `bob123`     | Customer |
| `charlie@ferrobank.local` | `charlie123` | Customer |
| `diana@ferrobank.local`   | `diana123`   | Customer |

The seed calls `AuthService::register` for each user, so passwords are hashed with
the same argon2id path production uses. It also creates demo accounts, loans,
transfers, and audit entries so the dashboards have something to show. Re-running
it is safe — existing rows are skipped.

*These dev seeds exist so teammates can log in without registering on every fresh
database. Remove them (or rotate the passwords) before any real deployment.*

---

## Development workflow

```bash
cargo fmt                       # before every commit
cargo clippy --all-targets      # CI runs this
cargo test                      # unit + integration tests
cargo sqlx prepare              # when you add new SQL queries
```

### Adding a migration

```bash
sqlx migrate add <descriptive_name>
# edit the generated file
sqlx migrate run
```

**Coordinate the migration number in the team chat before you write it** — see TEAM_CHARTER.md.

---

## Project structure

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the full layout and the contract every module follows.

```
src/
├── main.rs           # boot
├── routes.rs         # mounts every module
├── middleware/       # auth guard, session
├── models/           # one file per domain entity
├── services/         # business logic, trait-based
└── handlers/         # Actix routes, one file per module
templates/            # Askama, one folder per module
migrations/           # SQLx migrations, numbered
```

---

## Submission deliverables (spec v1.2)

| # | File | Format | Max size |
|---|---|---|---|
| 1 | Source code archive | `g##_source.zip` | 20 MB |
| 2 | Demo recording (15 min) | `g##_recording.mp4` | 200 MB |
| 3 | Presentation slides | `g##_slides.pptx` + `g##_slides.pdf` | 20 MB each |
| 4 | Project report (≤6 pages) | `g##_report.docx` + `g##_report.pdf` | 20 MB each |

Every file must show **Group Number, Student Name(s), Student ID(s) (SIT)** on the cover / title.
The report must explain each member's group contribution and their individual extended feature,
and should be informed by a brief literature review of real banking systems (Cyclos, Mambu,
open-source core banking projects) — see the Literature section in [TEAM_CHARTER.md](./TEAM_CHARTER.md).

---

## License

MIT — see LICENSE.
