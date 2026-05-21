# FerroBank

**Iron-clad core banking, built in Rust.**

A server-side-rendered enterprise banking platform built for the Web Programming module at SIT.
Demonstrates layered architecture, OOP via traits, ACID money movement, role-based access control, and audit logging.

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
| Auth | M2 | Registration, login, sessions, roles |
| Accounts | M3 | Open/close accounts, balances, account types |
| Transfers | M4 | ACID money movement, OTP confirm, audit log |
| Loans | M5 | Applications, amortization, repayments |
| Admin Dashboard | M1 (Platform Lead) | Cross-module read-only views & metrics |

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

# 5. Run the app
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

# 5. Run the app
cargo run
```

Open <http://localhost:8080>.

> If you have Git for Windows installed, you can use Git Bash and follow the
> **macOS** instructions verbatim (the `openssl`, `cp`, and `source` commands
> all work there).

### Default seeded users (created by 001_init.sql)

| Email | Password | Role |
|---|---|---|
| `admin@ferrobank.local` | `admin123` *(change me)* | Admin |
| `teller@ferrobank.local` | `teller123` | Teller |
| `alice@ferrobank.local` | `alice123` | Customer |

*Note: these seeds exist so teammates can develop without going through registration on every dev cycle. They are removed before any production deploy.*

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

## License

MIT — see LICENSE.
