//! Shared view helpers.
//!
//! Every Askama template that extends `layout.html` must include a
//! `layout: LayoutCtx` field. The base layout reads `layout.user` to render
//! the nav chip, sign-in button, etc.

use crate::middleware::auth::CurrentUser;

#[derive(Debug, Clone, Default)]
pub struct LayoutCtx {
    pub user: Option<UserChip>,
}

#[derive(Debug, Clone)]
pub struct UserChip {
    pub email: String,
    /// Lowercase role label: `"customer"`, `"teller"`, or `"admin"`.
    pub role: String,
}

impl LayoutCtx {
    /// Build from an optional current user. Public pages pass `None`.
    pub fn from_user(user: Option<&CurrentUser>) -> Self {
        Self {
            user: user.map(|u| UserChip {
                email: u.email.clone(),
                role: u.role.label().to_lowercase(),
            }),
        }
    }

    /// Convenience for public pages (no logged-in user expected).
    pub fn anonymous() -> Self {
        Self::default()
    }
}
