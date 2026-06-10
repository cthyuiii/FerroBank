//! Auth handlers — owned by the Auth module (Member 2).
//!
//! Flow:
//!   GET  /login     → render login form
//!   POST /login     → validate form, call AuthService::login, set session, redirect
//!   GET  /register  → render registration form
//!   POST /register  → validate, call AuthService::register, auto-login, redirect
//!   POST /logout    → purge session, redirect to /

use actix_session::Session;
use actix_web::{web, HttpResponse, Responder};
use askama::Template;
use serde::Deserialize;
use validator::Validate;

use crate::errors::AppError;
use crate::middleware::auth::{session, SessionUser};
use crate::models::user::{NewUser, Role};
use crate::services::auth_service::AuthService;
use crate::state::AppState;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    // NOTE: register these as plain top-level resources — do NOT wrap them in
    // `web::scope("")`. An empty-prefix scope matches *every* request path, and
    // because services are matched in registration order it would swallow the
    // later `/accounts`, `/transfers`, `/loans`, and `/admin` scopes and return
    // 404 for them (e.g. the post-login redirect to `/accounts`).
    cfg.route("/login", web::get().to(login_form))
        .route("/login", web::post().to(login_submit))
        .route("/register", web::get().to(register_form))
        .route("/register", web::post().to(register_submit))
        .route("/logout", web::post().to(logout));
}

// ── Templates ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    layout: LayoutCtx,
    error: Option<String>,
    /// Informational banner (account created / signed out for inactivity).
    notice: Option<String>,
    email: String,
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    registered: Option<u8>,
    expired: Option<u8>,
}

#[derive(Template)]
#[template(path = "auth/register.html")]
struct RegisterTemplate {
    layout: LayoutCtx,
    error: Option<String>,
    email: String,
    first_name: String,
    middle_name: String,
    last_name: String,
}

// ── Form payloads ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Validate)]
struct LoginForm {
    #[validate(email)]
    email: String,
    #[validate(length(min = 1, max = 128))]
    password: String,
}

#[derive(Debug, Deserialize, Validate)]
struct RegisterForm {
    #[validate(email)]
    email: String,
    #[validate(length(min = 1, max = 40))]
    first_name: String,
    /// Optional middle name.
    #[validate(length(max = 40))]
    middle_name: Option<String>,
    #[validate(length(min = 1, max = 40))]
    last_name: String,
    /// National ID — used by staff to verify identity during fraud reviews.
    #[validate(length(min = 5, max = 20, message = "must be 5-20 characters"))]
    nric: String,
    #[validate(length(min = 8, max = 128, message = "must be at least 8 characters"))]
    password: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn login_form(query: web::Query<LoginQuery>) -> Result<HttpResponse, AppError> {
    let notice = if query.registered.is_some() {
        Some("Account created — please sign in with your new credentials.".to_string())
    } else if query.expired.is_some() {
        Some("You were signed out after 5 minutes of inactivity. Please sign in again.".to_string())
    } else {
        None
    };
    render_login(None, notice, String::new())
}

async fn login_submit(
    form: web::Form<LoginForm>,
    svc: web::Data<dyn AuthService>,
    state: web::Data<AppState>,
    session: Session,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();

    if let Err(e) = form.validate() {
        return render_login(Some(format!("Please correct: {e}")), None, form.email);
    }

    let user = match svc.login(&form.email, &form.password).await {
        Ok(u) => u,
        Err(AppError::Unauthorized) => {
            return render_login(Some("Invalid email or password.".into()), None, form.email);
        }
        Err(other) => return Err(other),
    };

    session::login(
        &session,
        SessionUser {
            id: user.id,
            email: user.email.clone(),
            name: user.given_name(),
            role: user.role,
        },
    )?;

    // Customers who haven't linked Telegram yet are sent straight to the
    // linking page (the ActivityGuard enforces this on every later request).
    if user.role == Role::Customer && state.telegram_bot.is_some() {
        let linked: Option<bool> =
            sqlx::query_scalar::<_, bool>(r#"SELECT telegram_chat_id IS NOT NULL FROM users WHERE id = $1"#)
                .bind(user.id)
                .fetch_optional(&state.db)
                .await?;
        if !matches!(linked, Some(true)) {
            return Ok(redirect("/settings/telegram"));
        }
    }

    Ok(redirect(post_login_destination(user.role)))
}

async fn register_form() -> Result<HttpResponse, AppError> {
    render_register(None, String::new(), String::new(), String::new(), String::new())
}

async fn register_submit(
    form: web::Form<RegisterForm>,
    svc: web::Data<dyn AuthService>,
    _session: Session,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();

    if let Err(e) = form.validate() {
        return render_register(
            Some(format!("Please correct: {e}")),
            form.email,
            form.first_name,
            form.middle_name.unwrap_or_default(),
            form.last_name,
        );
    }

    let user = match svc
        .register(NewUser {
            email: form.email.clone(),
            password: form.password.clone(),
            first_name: form.first_name.trim().to_string(),
            middle_name: form
                .middle_name
                .clone()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
            last_name: form.last_name.trim().to_string(),
            nric: Some(form.nric.trim().to_string()),
            role: Role::Customer, // all self-registrations are customers; staff are seeded
        })
        .await
    {
        Ok(u) => u,
        Err(AppError::Conflict(msg)) => {
            return render_register(
                Some(msg),
                form.email,
                form.first_name,
                form.middle_name.unwrap_or_default(),
                form.last_name,
            );
        }
        Err(other) => return Err(other),
    };

    // No auto-login: the user must sign in with their new credentials —
    // verifying the password they just set before any session exists.
    tracing::info!(user_id = user.id, "registration complete; fresh sign-in required");
    Ok(redirect("/login?registered=1"))
}

async fn logout(session: Session) -> impl Responder {
    session::logout(&session);
    redirect("/")
}

// ── Helpers ──────────────────────────────────────────────────────────

fn render_login(
    error: Option<String>,
    notice: Option<String>,
    email: String,
) -> Result<HttpResponse, AppError> {
    let body = LoginTemplate {
        layout: LayoutCtx::anonymous(),
        error,
        notice,
        email,
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("login template: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn render_register(
    error: Option<String>,
    email: String,
    first_name: String,
    middle_name: String,
    last_name: String,
) -> Result<HttpResponse, AppError> {
    let body = RegisterTemplate {
        layout: LayoutCtx::anonymous(),
        error,
        email,
        first_name,
        middle_name,
        last_name,
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("register template: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

fn redirect(location: &str) -> HttpResponse {
    HttpResponse::Found()
        .insert_header(("Location", location))
        .finish()
}

/// Where each role lands after signing in.
/// - Admins get the admin dashboard.
/// - Tellers work the loan queue (they're not allowed in the admin scope, so
///   sending them to `/admin/dashboard` would 403).
/// - Customers go to their accounts.
fn post_login_destination(role: Role) -> &'static str {
    match role {
        Role::Admin => "/admin/dashboard",
        Role::Teller => "/loans",
        Role::Customer => "/accounts",
    }
}
