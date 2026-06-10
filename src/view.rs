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
    /// Given name, for greetings.
    pub name: String,
    /// Lowercase role label: `"customer"`, `"teller"`, or `"admin"`.
    pub role: String,
}

impl LayoutCtx {
    /// Build from an optional current user. Public pages pass `None`.
    pub fn from_user(user: Option<&CurrentUser>) -> Self {
        Self {
            user: user.map(|u| UserChip {
                email: u.email.clone(),
                name: u.name.clone(),
                role: u.role.label().to_lowercase(),
            }),
        }
    }

    /// Convenience for public pages (no logged-in user expected).
    pub fn anonymous() -> Self {
        Self::default()
    }
}

/// Shared "enter your one-time code" page, reused by every OTP-gated action
/// (account opening, loan applications, profile changes). The transfer flow
/// has its own confirm page because it shows richer transfer details.
#[derive(askama::Template)]
#[template(path = "otp_confirm.html")]
pub struct OtpConfirmPage {
    pub layout: LayoutCtx,
    pub title: String,
    /// (label, value) rows summarizing what's about to happen.
    pub summary: Vec<(String, String)>,
    pub action_url: String,
    pub cancel_url: String,
    pub action_id: i64,
    /// `Some(code)` → on-screen fallback; `None` → sent via Telegram.
    pub demo_otp: Option<String>,
    pub error: Option<String>,
}
