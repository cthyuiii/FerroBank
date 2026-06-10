//! Shared application state injected into every handler.
//!
//! Pull it into a handler with `data: web::Data<AppState>`.

use crate::config::Config;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// Telegram bot username (from `getMe`), used to build t.me deep links.
    /// `None` when Telegram OTP delivery isn't configured.
    pub telegram_bot: Option<String>,
}
