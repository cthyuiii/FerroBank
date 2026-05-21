//! Cross-cutting Actix middleware and request extractors.

pub mod auth;

pub use auth::{CurrentUser, RequireRole, SessionUser};
