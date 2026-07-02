//! FerroBank - iron-clad core banking, built in Rust.
//!
//! The crate is split into:
//! - [`config`] / [`db`] / [`state`] - platform infrastructure
//! - [`errors`] - the shared `AppError` type every layer returns
//! - [`middleware`] - auth guard, current-user extractor
//! - [`models`] - domain entities (one file per domain)
//! - [`services`] - business logic behind traits (one file per domain)
//! - [`handlers`] - Actix routes (one file per domain)
//! - [`routes`] - the one place that mounts every module's routes

pub mod config;
pub mod db;
pub mod errors;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod routes;
pub mod services;
pub mod state;
pub mod view;
