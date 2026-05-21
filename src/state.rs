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
}
