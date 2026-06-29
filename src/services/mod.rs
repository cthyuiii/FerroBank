//! Business logic, organized as one file per domain.
//!
//! Each module exposes a trait + at least one concrete impl. Handlers depend on
//! the trait (via `web::Data<dyn XxxService>`), never the concrete type for polymorphism.

pub mod account_service;
pub mod action_otp_service;
pub mod admin_service;
pub mod audit_service;
pub mod auth_service;
pub mod loan_service;
pub mod telegram_service;
pub mod transfer_service;
