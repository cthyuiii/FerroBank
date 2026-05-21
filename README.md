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

### Prerequisites

- Rust 1.78+ (`rustup toolchain install stable`)
- Docker + Docker Compose (for the local Postgres)
- `sqlx-cli`: `cargo install sqlx-cli --no-default-features --features postgres,rustls`

### Run it

```bash
# 1. Start Postgres
docker compose up -d db

# 2. Set up env
cp .env.example .env

# 3. Create the database and apply migrations
sqlx database create
sqlx migrate run

# 4. Run the app
cargo run
```

Open <http://localhost:8080>.

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
