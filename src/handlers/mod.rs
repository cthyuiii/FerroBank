//! Actix route handlers
//!
//! Every module exports a `pub fn routes(cfg: &mut web::ServiceConfig)` function.
//! Only `src/routes.rs` calls those 

pub mod accounts;
pub mod admin;
pub mod auth;
pub mod home;
pub mod loans;
pub mod settings;
pub mod transfers;
