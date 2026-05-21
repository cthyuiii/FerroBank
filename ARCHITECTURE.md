# FerroBank — Architecture

This document is the reference for **how the pieces fit together**. Read it before writing your first module file.

---

## High-level

```
Browser
  ↓ HTTP / form POST
[ Actix middleware ]   ← session, auth guard, logging
  ↓
[ Handler (Actix route) ]   ← parses form, calls service, picks template
  ↓                              ↘
[ Service trait ]                  [ Askama template ]
  ↓                              ↙
[ Model / SQLx query ]
  ↓
PostgreSQL
```

A request enters at the top, passes through middleware that injects the current user, lands in a handler. The handler is **thin** — it parses input, calls a service method, and renders a template. The service is **thick** — that's where business rules live. The model layer is just SQLx-typed structs and queries.

---

## Directory layout

```
ferrobank/
├── Cargo.toml
├── .env.example
├── Dockerfile
├── docker-compose.yml          # Postgres for local dev
├── README.md
├── ARCHITECTURE.md             # this file
├── TEAM_CHARTER.md
├── migrations/                 # SQLx migrations, run via sqlx-cli
│   └── 001_init.sql
├── static/                     # css, js, images
├── templates/                  # Askama templates
│   ├── layout.html             # base layout, every page extends this
│   ├── partials/
│   ├── auth/
│   ├── accounts/
│   ├── transfers/
│   ├── loans/
│   └── admin/
└── src/
    ├── main.rs                 # boot: load config, open pool, mount routes
    ├── config.rs               # env loading
    ├── db.rs                   # PgPool construction
    ├── state.rs                # AppState shared via web::Data
    ├── errors.rs               # AppError enum + ResponseError impl
    ├── routes.rs               # mounts every module's routes()
    ├── middleware/
    │   ├── mod.rs
    │   └── auth.rs             # CurrentUser extractor, RequireRole guard
    ├── models/
    │   ├── mod.rs
    │   ├── user.rs             # Auth owner
    │   ├── account.rs          # Accounts owner
    │   ├── transfer.rs         # Transfers owner
    │   └── loan.rs             # Loans owner
    ├── services/
    │   ├── mod.rs
    │   ├── auth_service.rs
    │   ├── account_service.rs
    │   ├── transfer_service.rs
    │   ├── audit_service.rs
    │   ├── loan_service.rs
    │   └── admin_service.rs
    └── handlers/
        ├── mod.rs
        ├── home.rs             # Platform Lead, the `/` landing
        ├── auth.rs
        ├── accounts.rs
        ├── transfers.rs
        ├── loans.rs
        └── admin.rs
```

---

## What the Platform Lead publishes

These are the types and functions teammates import. Stable surface — anything else is internal.

### `AppState`
```rust
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
}
```
Available in any handler via `data: web::Data<AppState>`.

### `AppError`
```rust
pub enum AppError {
    NotFound(String),
    Unauthorized,
    Forbidden,
    BadRequest(String),
    Conflict(String),       // e.g., insufficient funds
    Internal(anyhow::Error),
}
```
Implements `actix_web::ResponseError` so returning it from a handler renders the right HTTP status and an error page. Has `From<sqlx::Error>` and `From<anyhow::Error>` so `?` just works.

### `CurrentUser`
```rust
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub role: Role,
}
```
An Actix `FromRequest` extractor. Put it in any handler signature where you need the logged-in user:
```rust
async fn my_handler(user: CurrentUser, ...) -> Result<HttpResponse, AppError> { ... }
```
If the session cookie is missing or invalid, the extractor returns `AppError::Unauthorized`, which redirects to `/login`.

### `RequireRole`
```rust
pub struct RequireRole(pub Role);
```
A guard you compose around admin-only handlers:
```rust
cfg.service(
    web::scope("/admin")
        .guard(RequireRole(Role::Admin))
        .route("/dashboard", web::get().to(handlers::admin::dashboard))
);
```

---

## How a module plugs in

Each module owner writes their handlers, services, models, and templates. Then they make **one** change to expose their routes:

In `src/handlers/<your_module>.rs`, define and export:

```rust
pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/accounts")
            .route("", web::get().to(list))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/{id}", web::get().to(detail)),
    );
}
```

The Platform Lead's `src/routes.rs` mounts everything:

```rust
pub fn configure(cfg: &mut web::ServiceConfig) {
    handlers::home::routes(cfg);
    handlers::auth::routes(cfg);
    handlers::accounts::routes(cfg);
    handlers::transfers::routes(cfg);
    handlers::loans::routes(cfg);
    handlers::admin::routes(cfg);
}
```

You never edit `routes.rs` (except the Platform Lead). You only add your handler to the `pub use` chain in `src/handlers/mod.rs` and declare your module in `src/services/mod.rs` and `src/models/mod.rs`.

---

## How services work (the OOP part)

Every domain operation lives behind a trait, not bare functions. Example:

```rust
// src/services/account_service.rs
#[async_trait::async_trait]
pub trait AccountService: Send + Sync {
    async fn open_account(&self, user_id: i64, kind: AccountType) -> Result<Account, AppError>;
    async fn get_balance(&self, account_id: i64) -> Result<Decimal, AppError>;
    async fn list_for_user(&self, user_id: i64) -> Result<Vec<Account>, AppError>;
}

pub struct PgAccountService { pub db: PgPool }

#[async_trait::async_trait]
impl AccountService for PgAccountService {
    async fn open_account(&self, user_id: i64, kind: AccountType) -> Result<Account, AppError> {
        // sqlx::query_as!(...)
    }
    // ...
}
```

In `main.rs`, the Platform Lead builds and registers the concrete impl:

```rust
let account_service: Arc<dyn AccountService> = Arc::new(PgAccountService { db: pool.clone() });
App::new()
    .app_data(web::Data::from(account_service))
    // ...
```

Handlers then pull it in by trait, not concrete type:

```rust
async fn list(
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let accounts = svc.list_for_user(user.id).await?;
    // render template
}
```

This pattern gives you:

- **Polymorphism** — swap `PgAccountService` for `MockAccountService` in unit tests
- **Encapsulation** — handlers know nothing about SQL
- **Open/closed** — adding a `SqliteAccountService` later doesn't touch handlers
- A clean answer when the grader asks "where's the OOP?"

---

## Database conventions

- Tables in `snake_case`, plural (`accounts`, `transfers`).
- Every table has `id BIGSERIAL PRIMARY KEY` and `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`.
- Foreign keys are `<table>_id BIGINT NOT NULL REFERENCES <table>(id)`.
- Money columns are `NUMERIC(18,2) NOT NULL`. Map to `rust_decimal::Decimal`.
- Use `sqlx::query!` and `sqlx::query_as!` macros — compile-time SQL checking against the real schema.

---

## Template conventions

Every page:

```html
{% extends "layout.html" %}

{% block title %}Accounts · FerroBank{% endblock %}

{% block content %}
  <h1>Your accounts</h1>
  ...
{% endblock %}
```

The base layout provides nav, current-user chip, flash messages, and the Tailwind CDN tag. Don't reinvent it per module.

---

## Local development

```bash
docker compose up -d db          # starts Postgres on localhost:5432
cp .env.example .env
sqlx database create
sqlx migrate run
cargo run
```

Open <http://localhost:8080>.

The pre-prepared SQLx offline cache: when you add new SQL queries, run `cargo sqlx prepare` and commit the resulting `.sqlx/` directory so CI can build without a live DB.
