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
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("")
            .route("/login", web::get().to(login_form))
            .route("/login", web::post().to(login_submit))
            .route("/register", web::get().to(register_form))
            .route("/register", web::post().to(register_submit))
            .route("/logout", web::post().to(logout)),
    );
}

// ── Templates ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    layout: LayoutCtx,
    error: Option<String>,
    email: String,
}

#[derive(Template)]
#[template(path = "auth/register.html")]
struct RegisterTemplate {
    layout: LayoutCtx,
    error: Option<String>,
    email: String,
    full_name: String,
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
    #[validate(length(min = 2, max = 80))]
    full_name: String,
    #[validate(length(min = 8, max = 128, message = "must be at least 8 characters"))]
    password: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn login_form() -> Result<HttpResponse, AppError> {
    render_login(None, String::new())
}

async fn login_submit(
    form: web::Form<LoginForm>,
    svc: web::Data<dyn AuthService>,
    session: Session,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();

    if let Err(e) = form.validate() {
        return render_login(
            Some(format!("Please correct: {e}")),
            form.email,
        );
    }

    let user = match svc.login(&form.email, &form.password).await {
        Ok(u) => u,
        Err(AppError::Unauthorized) => {
            return render_login(
                Some("Invalid email or password.".into()),
                form.email,
            );
        }
        Err(other) => return Err(other),
    };

    session::login(
        &session,
        SessionUser {
            id: user.id,
            email: user.email.clone(),
            role: user.role,
        },
    )?;

    Ok(redirect(post_login_destination(user.role)))
}

async fn register_form() -> Result<HttpResponse, AppError> {
    render_register(None, String::new(), String::new())
}

async fn register_submit(
    form: web::Form<RegisterForm>,
    svc: web::Data<dyn AuthService>,
    session: Session,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();

    if let Err(e) = form.validate() {
        return render_register(
            Some(format!("Please correct: {e}")),
            form.email,
            form.full_name,
        );
    }

    let user = match svc
        .register(NewUser {
            email: form.email.clone(),
            password: form.password.clone(),
            full_name: form.full_name.clone(),
            role: Role::Customer, // all self-registrations are customers; staff are seeded
        })
        .await
    {
        Ok(u) => u,
        Err(AppError::Conflict(msg)) => {
            return render_register(Some(msg), form.email, form.full_name);
        }
        Err(other) => return Err(other),
    };

    // Auto-login after successful registration.
    session::login(
        &session,
        SessionUser {
            id: user.id,
            email: user.email.clone(),
            role: user.role,
        },
    )?;

    Ok(redirect(post_login_destination(user.role)))
}

async fn logout(session: Session) -> impl Responder {
    session::logout(&session);
    redirect("/")
}

// ── Helpers ──────────────────────────────────────────────────────────

fn render_login(error: Option<String>, email: String) -> Result<HttpResponse, AppError> {
    let body = LoginTemplate {
        layout: LayoutCtx::anonymous(),
        error,
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
    full_name: String,
) -> Result<HttpResponse, AppError> {
    let body = RegisterTemplate {
        layout: LayoutCtx::anonymous(),
        error,
        email,
        full_name,
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

/// Customers land on their own accounts page; staff land on the admin dashboard.
fn post_login_destination(role: Role) -> &'static str {
    match role {
        Role::Admin | Role::Teller => "/admin/dashboard",
        Role::Customer => "/accounts",
    }
}
