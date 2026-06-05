//! FerroBank binary entrypoint.
//!
//! All logic lives in the `ferrobank` library crate (see `src/lib.rs`).
//! This file does three things:
//!   1. Loads configuration from the environment.
//!   2. Connects to Postgres and runs migrations.
//!   3. Builds the Actix `App`, wires services, and serves HTTP.

use std::sync::Arc;

use actix_session::{storage::CookieSessionStore, SessionMiddleware};
use actix_web::{cookie::Key, web, App, HttpServer};
use tracing_actix_web::TracingLogger;

use ferrobank::{
    config::Config,
    db, routes,
    services::{
        account_service::{AccountService, PgAccountService},
        admin_service::{AdminService, PgAdminService},
        audit_service::{AuditService, PgAuditService},
        auth_service::{AuthService, PgAuthService},
        loan_service::{LoanService, PgLoanService},
        transfer_service::{PgTransferService, TransferService},
    },
    state::AppState,
};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    // 1. Environment
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("ferrobank=info,actix_web=info")
            }),
        )
        .init();

    let config = Arc::new(Config::from_env()?);

    // 2. Database
    tracing::info!("connecting to database");
    let pool = db::connect(&config.database_url).await?;

    tracing::info!("running migrations");
    sqlx::migrate!("./migrations").run(&pool).await?;

    // 3. Shared application state
    let state = web::Data::new(AppState {
        db: pool.clone(),
        config: config.clone(),
    });

    // ── Service wiring ───────────────────────────────────────────────────
    // Each module owner provides `pub struct PgXxxService` and the matching
    // `impl XxxService for PgXxxService`. The Platform Lead just instantiates
    // them here once — there are no per-request constructors anywhere.
    //
    // Teammates: do NOT add new lines to main.rs when you fill in your service.
    // Just implement the `new(...)` constructor with the signature shown.
    let auth_service: Arc<dyn AuthService> = Arc::new(PgAuthService::new(pool.clone()));
    let account_service: Arc<dyn AccountService> = Arc::new(PgAccountService::new(pool.clone()));
    let audit_service: Arc<dyn AuditService> = Arc::new(PgAuditService::new(pool.clone()));
    let transfer_service: Arc<dyn TransferService> = Arc::new(PgTransferService::new(
        pool.clone(),
        audit_service.clone(),
    ));
    let loan_service: Arc<dyn LoanService> = Arc::new(PgLoanService::new(pool.clone()));
    // Admin service composes the others. Build it last and inject the deps.
    let admin_service: Arc<dyn AdminService> = Arc::new(
        PgAdminService::new(pool.clone()).with_services(
            account_service.clone(),
            loan_service.clone(),
            audit_service.clone(),
        ),
    );

    // Wrap each Arc<dyn Trait> in actix's web::Data so handlers can extract it.
    let auth_data = web::Data::from(auth_service);
    let account_data = web::Data::from(account_service);
    let transfer_data = web::Data::from(transfer_service);
    let loan_data = web::Data::from(loan_service);
    let admin_data = web::Data::from(admin_service);
    let audit_data = web::Data::from(audit_service);

    // ── HTTP server ──────────────────────────────────────────────────────
    let session_key = Key::from(config.session_secret.as_bytes());
    let bind_host = config.app_host.clone();
    let bind_port = config.app_port;

    tracing::info!(host = %bind_host, port = bind_port, "starting FerroBank");

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(auth_data.clone())
            .app_data(account_data.clone())
            .app_data(transfer_data.clone())
            .app_data(loan_data.clone())
            .app_data(admin_data.clone())
            .app_data(audit_data.clone())
            .wrap(TracingLogger::default())
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .cookie_name("ferrobank_session".to_string())
                    .cookie_secure(false) // flip to true behind HTTPS in production
                    .cookie_http_only(true)
                    .build(),
            )
            .service(actix_files::Files::new("/static", "./static"))
            .configure(routes::configure)
    })
    .bind((bind_host, bind_port))?
    .run()
    .await?;

    Ok(())
}
