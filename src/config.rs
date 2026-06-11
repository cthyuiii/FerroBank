//! Environment-driven application configuration.

use anyhow::Context;

/// Runtime configuration, loaded from environment variables on startup.
///
/// Mutating env vars after the app starts has no effect - this is read once.
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub app_host: String,
    pub app_port: u16,
    pub session_secret: String,
    /// Telegram bot token for OTP delivery. `None` → codes show on screen.
    pub telegram_bot_token: Option<String>,
}

impl Config {
    /// Load configuration from the environment (and `.env` if present).
    ///
    /// Returns an error with a helpful message if anything required is missing
    /// or malformed.
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL must be set (see .env.example)")?;

        let app_host = std::env::var("APP_HOST").unwrap_or_else(|_| "127.0.0.1".into());

        let app_port = std::env::var("APP_PORT")
            .ok()
            .map(|p| p.parse::<u16>())
            .transpose()
            .context("APP_PORT must be a valid port number")?
            .unwrap_or(8080);

        let session_secret = std::env::var("SESSION_SECRET")
            .context("SESSION_SECRET must be set (generate with: openssl rand -base64 64)")?;

        if session_secret.as_bytes().len() < 64 {
            anyhow::bail!(
                "SESSION_SECRET must be at least 64 bytes (got {}). \
                 Generate one with: openssl rand -base64 64",
                session_secret.as_bytes().len()
            );
        }

        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());

        Ok(Self {
            database_url,
            app_host,
            app_port,
            session_secret,
            telegram_bot_token,
        })
    }
}
