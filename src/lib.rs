//! FerroBank - iron-clad core banking, built in Rust.
//!
//! The crate is split into:
//! - [`config`] / [`db`] / [`state`] - platform infrastructure (Platform Lead)
//! - [`errors`] - shared `AppError` everyone returns (Platform Lead)
//! - [`middleware`] - auth guard, current-user extractor (Platform Lead)
//! - [`models`] - domain entities (one file per module owner)
//! - [`services`] - business logic behind traits (one file per module owner)
//! - [`handlers`] - Actix routes (one file per module owner)
//! - [`routes`] - the one place that mounts every module's routes
//!
//! See `ARCHITECTURE.md` for the contract every module follows.

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
