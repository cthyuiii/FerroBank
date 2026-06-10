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

use sqlx::Connection as _;

use ferrobank::{
    config::Config,
    db,
    middleware::auth::ActivityGuard,
    routes,
    services::{
        account_service::{AccountService, PgAccountService},
        action_otp_service::{ActionOtpService, PgActionOtpService},
        admin_service::{AdminService, PgAdminService},
        audit_service::{AuditService, PgAuditService},
        auth_service::{AuthService, PgAuthService},
        loan_service::{LoanService, PgLoanService},
        telegram_service::{self, OtpChannel, ScreenOtp, TelegramOtp},
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

    // 3. Telegram (optional): resolve the bot's username once for deep links.
    let telegram_bot = match &config.telegram_bot_token {
        Some(token) => telegram_service::bot_username(token).await,
        None => None,
    };
    match &telegram_bot {
        Some(bot) => tracing::info!(bot = %bot, "telegram OTP delivery enabled"),
        None => tracing::info!("telegram OTP not configured — codes shown on screen"),
    }

    // 4. Shared application state
    let state = web::Data::new(AppState {
        db: pool.clone(),
        config: config.clone(),
        telegram_bot,
    });

    // ── Service wiring ───────────────────────────────────────────────────
    // Each module owner provides `pub struct PgXxxService` and the matching
    // `impl XxxService for PgXxxService`. The Platform Lead just instantiates
    // them here once — there are no per-request constructors anywhere.
    //
    // Teammates: do NOT add new lines to main.rs when you fill in your service.
    // Just implement the `new(...)` constructor with the signature shown.
    // OTP channel: Telegram when configured, on-screen fallback otherwise.
    // The poller is the background task that completes /start account links.
    let otp_channel: Arc<dyn OtpChannel> = match &config.telegram_bot_token {
        Some(token) => {
            tokio::spawn(telegram_service::run_link_poller(pool.clone(), token.clone()));
            Arc::new(TelegramOtp::new(pool.clone(), token))
        }
        None => Arc::new(ScreenOtp),
    };

    let auth_service: Arc<dyn AuthService> = Arc::new(PgAuthService::new(pool.clone()));
    let account_service: Arc<dyn AccountService> = Arc::new(PgAccountService::new(pool.clone()));
    let audit_service: Arc<dyn AuditService> = Arc::new(PgAuditService::new(pool.clone()));
    let transfer_service: Arc<dyn TransferService> = Arc::new(PgTransferService::new(
        pool.clone(),
        audit_service.clone(),
        otp_channel.clone(),
    ));
    let loan_service: Arc<dyn LoanService> =
        Arc::new(PgLoanService::new(pool.clone(), otp_channel.clone()));
    // Generalized OTP guard for account opening / loan applications / profile changes.
    let action_otp_service: Arc<dyn ActionOtpService> =
        Arc::new(PgActionOtpService::new(pool.clone(), otp_channel.clone()));
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
    let action_otp_data = web::Data::from(action_otp_service);

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
            .app_data(action_otp_data.clone())
            .wrap(TracingLogger::default())
            // Runs after the session middleware: activity trail, 5-minute
            // inactivity TTL (database time), mandatory Telegram linking.
            .wrap(ActivityGuard)
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .cookie_name("ferrobank_session".to_string())
                    .cookie_secure(false) // flip to true behind HTTPS in production
                    .cookie_http_only(true)
                    .build(),
            )
            .configure(routes::configure)
    })
    .bind((bind_host, bind_port))?
    .run()
    .await?;

    // ── Graceful-shutdown snapshot ──────────────────────────────────────
    // `run()` only returns once actix has finished a graceful shutdown
    // (Ctrl-C / SIGTERM), so this is the server's final act: append a
    // whole-bank state snapshot to the audit log.
    // Use a dedicated connection: the shared pool's connections lived on the
    // (now stopped) worker runtimes, so acquiring from it can time out here.
    tracing::info!("server stopped — writing shutdown snapshot to audit_log");
    match sqlx::postgres::PgConnection::connect(&config.database_url).await {
        Ok(mut conn) => {
            if let Err(e) =
                ferrobank::services::audit_service::snapshot_system_state(&mut conn).await
            {
                tracing::error!(error = ?e, "failed to write shutdown snapshot");
            }
            let _ = conn.close().await;
        }
        Err(e) => tracing::error!(error = ?e, "could not connect for shutdown snapshot"),
    }

    Ok(())
}
