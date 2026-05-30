//! Dev-only seed script: creates three demo users using the real AuthService.
//!
//! Usage:
//!   cargo run --bin seed
//!
//! Idempotent — re-running skips users that already exist. Safe to run after
//! every fresh `sqlx database create` + `sqlx migrate run`.
//!
//! Owner: Member 2 (Auth). Member 2 may extend this to seed additional
//! demo data (sample accounts, sample transfers) once those modules are in.

use std::sync::Arc;

use ferrobank::{
    config::Config,
    db,
    errors::AppError,
    models::user::{NewUser, Role},
    services::auth_service::{AuthService, PgAuthService},
};

#[derive(Debug)]
struct SeedUser {
    email: &'static str,
    password: &'static str,
    full_name: &'static str,
    role: Role,
}

const SEED_USERS: &[SeedUser] = &[
    SeedUser {
        email: "admin@ferrobank.local",
        password: "admin123",
        full_name: "Admin User",
        role: Role::Admin,
    },
    SeedUser {
        email: "teller@ferrobank.local",
        password: "teller123",
        full_name: "Teller User",
        role: Role::Teller,
    },
    SeedUser {
        email: "alice@ferrobank.local",
        password: "alice123",
        full_name: "Alice Smith",
        role: Role::Customer,
    },
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(Config::from_env()?);
    let pool = db::connect(&config.database_url).await?;
    let auth = PgAuthService::new(pool);

    println!("Seeding {} demo users...", SEED_USERS.len());

    for seed in SEED_USERS {
        let result = auth
            .register(NewUser {
                email: seed.email.to_string(),
                password: seed.password.to_string(),
                full_name: seed.full_name.to_string(),
                role: seed.role,
            })
            .await;

        match result {
            Ok(user) => {
                println!(
                    "  ✓ created {} ({}) — password: {}",
                    user.email,
                    seed.role.label(),
                    seed.password
                );
            }
            Err(AppError::Conflict(_)) => {
                println!("  · {} already exists, skipping", seed.email);
            }
            Err(e) => {
                eprintln!("  ✗ failed to create {}: {e}", seed.email);
                return Err(anyhow::anyhow!(e.to_string()));
            }
        }
    }

    println!("\nDone. Sign in at http://localhost:8080/login");
    Ok(())
}
