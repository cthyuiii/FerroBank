//! Top-level route mounting. The **only** place that calls into each module's `routes()`.
//!
//! New modules register their `routes()` here; routes are never mounted
//! anywhere else, so this file is the complete route map of the application.

use actix_web::web;

use crate::handlers;

pub fn configure(cfg: &mut web::ServiceConfig) {
    handlers::home::routes(cfg);
    handlers::auth::routes(cfg);
    handlers::accounts::routes(cfg);
    handlers::transfers::routes(cfg);
    handlers::loans::routes(cfg);
    handlers::settings::routes(cfg);
    handlers::admin::routes(cfg);
}
