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
| Auth | M2 | Registration, login, sessions, roles, account lockout |
| Accounts | M3 | Open/close accounts, balances, account types, PDF statements |
| Transfers | M4 | ACID money movement, **Mutex + row-lock concurrency**, OTP confirm, fraud rules, audit log |
| Loans & Fixed Deposits | M5 | Loan applications, amortization, repayments, fixed-deposit accrual |
| Admin Dashboard | M1 (Platform Lead) | Cross-module read-only views, audit-log explorer |

See **[TEAM_CHARTER.md](./TEAM_CHARTER.md)** for each member's group baseline and their individual extended feature (which together cover the 60% group + 40% individual marking criteria).

---

## Quick start

Pick the section that matches your OS — the steps are otherwise identical.

### macOS

#### One-time install

```bash
# Rust toolchain (skip if you already have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Docker Desktop (provides the Postgres container)
brew install --cask docker
open -a Docker            # launch once so the daemon starts

# SQLx CLI for migrations
cargo install sqlx-cli --no-default-features --features postgres,rustls
```

#### Run

```bash
# 1. Start Postgres in the background
docker compose up -d db

# 2. Set up env
cp .env.example .env

# 3. Generate a strong session secret and paste it into .env (SESSION_SECRET=...)
openssl rand -base64 64

# 4. Create the database and apply migrations
sqlx database create
sqlx migrate run

# 5. Seed three demo users (admin / teller / customer)
cargo run --bin seed

# 6. Run the app
cargo run
```

Open <http://localhost:8080>.

---

### Windows (PowerShell)

Open **PowerShell 7+** (or Windows PowerShell 5.1) — not Command Prompt.

#### One-time install

```powershell
# Rust toolchain (skip if you already have it)
winget install Rustlang.Rustup
# Restart the shell so cargo is on PATH.

# Docker Desktop (provides the Postgres container)
winget install Docker.DockerDesktop
# Launch Docker Desktop once from the Start Menu so the daemon starts.

# SQLx CLI for migrations
cargo install sqlx-cli --no-default-features --features postgres,rustls
```

> If `winget` isn't available, download installers from
> <https://rustup.rs/> and <https://www.docker.com/products/docker-desktop/>.

#### Run

```powershell
# 1. Start Postgres in the background
docker compose up -d db

# 2. Set up env
Copy-Item .env.example .env

# 3. Generate a strong session secret and paste it into .env (SESSION_SECRET=...)
$bytes = New-Object byte[] 64
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
[Convert]::ToBase64String($bytes)

# 4. Create the database and apply migrations
sqlx database create
sqlx migrate run

# 5. Seed three demo users (admin / teller / customer)
cargo run --bin seed

# 6. Run the app
cargo run
```

Open <http://localhost:8080>.

> If you have Git for Windows installed, you can use Git Bash and follow the
> **macOS** instructions verbatim (the `openssl`, `cp`, and `source` commands
> all work there).

### Default seeded users (created by `cargo run --bin seed`)

| Email | Password | Role |
|---|---|---|
| `admin@ferrobank.local` | `admin123` *(change me)* | Admin |
| `teller@ferrobank.local` | `teller123` | Teller |
| `alice@ferrobank.local`  | `alice123`  | Customer |

The seed binary calls `AuthService::register` for each user, so the passwords are
hashed with the same argon2id code path that production uses. Re-running the
seed is safe — existing users are skipped.

*These dev seeds exist so teammates can log in without going through registration
on every fresh database. They should be removed (or have their passwords rotated)
before any real deployment.*

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
