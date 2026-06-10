//! Cross-cutting Actix middleware and request extractors.

pub mod auth;

pub use auth::{ActivityGuard, CurrentUser, RequireRole, SessionUser};
