//! Top-level route mounting. The **only** place that calls into each module's `routes()`.
//!
//! Module owners: add your `routes()` call here when your module is ready. Don't
//! mount routes anywhere else.

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
