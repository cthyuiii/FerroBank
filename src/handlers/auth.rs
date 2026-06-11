//! Auth handlers - owned by the Auth module (Member 2).
//!
//! Flow:
//!   GET  /login     → render login form
//!   POST /login     → validate form, call AuthService::login, set session, redirect
//!   GET  /register  → render registration form
//!   POST /register  → validate, call AuthService::register, auto-login, redirect
//!   POST /logout    → purge session, redirect to /

use actix_session::Session;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use askama::Template;
use serde::Deserialize;
use validator::Validate;

use crate::errors::AppError;
use crate::middleware::auth::{session, SessionUser};
use crate::models::user::{NewUser, Role};
use crate::services::action_otp_service::ActionOtpService;
use crate::services::auth_service::{
    block_origin, login_origin_is_new, origin_blocked, record_login_session, AuthService,
};
use crate::services::telegram_service::OtpChannel;
use crate::state::AppState;
use crate::view::{LayoutCtx, OtpConfirmPage};

pub fn routes(cfg: &mut web::ServiceConfig) {
    // NOTE: register these as plain top-level resources - do NOT wrap them in
    // `web::scope("")`. An empty-prefix scope matches *every* request path, and
    // because services are matched in registration order it would swallow the
    // later `/accounts`, `/transfers`, `/loans`, and `/admin` scopes and return
    // 404 for them (e.g. the post-login redirect to `/accounts`).
    cfg.route("/login", web::get().to(login_form))
        .route("/login", web::post().to(login_submit))
        .route("/login/stepup", web::post().to(login_stepup))
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
    /// National ID - used by staff to verify identity during fraud reviews.
    #[validate(length(min = 5, max = 20, message = "must be 5-20 characters"))]
    nric: String,
    #[validate(length(min = 8, max = 128, message = "must be at least 8 characters"))]
    password: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn login_form(query: web::Query<LoginQuery>) -> Result<HttpResponse, AppError> {
    let notice = if query.registered.is_some() {
        Some("Account created - please sign in with your new credentials.".to_string())
    } else if query.expired.is_some() {
        Some("You were signed out after 5 minutes of inactivity. Please sign in again.".to_string())
    } else {
        None
    };
    render_login(None, notice, String::new())
}

async fn login_submit(
    req: HttpRequest,
    form: web::Form<LoginForm>,
    svc: web::Data<dyn AuthService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    otp_channel: web::Data<dyn OtpChannel>,
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

    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let ip = req
        .connection_info()
        .realip_remote_addr()
        .unwrap_or("unknown")
        .to_string();

    // A blocked origin (3 failed step-up codes) may not sign in at all,
    // even with the right password, until the block expires.
    if origin_blocked(&state.db, user.id, &user_agent, &ip).await {
        return render_login(
            Some(
                "Sign-in from this device/network is temporarily blocked after repeated failed verification attempts. Try again later, or sign in from a device you've used before.".into(),
            ),
            None,
            form.email,
        );
    }

    // Apply any matured 24h bot-scheduled unlink before checking link state.
    let _ = sqlx::query(
        r#"UPDATE users SET telegram_chat_id = NULL, telegram_unlink_at = NULL
           WHERE id = $1 AND telegram_unlink_at IS NOT NULL AND telegram_unlink_at <= now()"#,
    )
    .bind(user.id)
    .execute(&state.db)
    .await;

    // ── Risk-based step-up ──────────────────────────────────────────────
    // Password alone is enough from a known origin. A first-seen browser or
    // network must ALSO present a one-time code before any session exists.
    // Only possible for linked customers (first login is exempt by design:
    // no history yet, and unlinked users have no out-of-band channel).
    if user.role == Role::Customer && state.telegram_bot.is_some() {
        let linked: Option<bool> = sqlx::query_scalar::<_, bool>(
            r#"SELECT telegram_chat_id IS NOT NULL FROM users WHERE id = $1"#,
        )
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?;
        if matches!(linked, Some(true))
            && login_origin_is_new(&state.db, user.id, &user_agent, &ip).await
        {
            let challenge = otp_svc
                .begin(
                    user.id,
                    "login.stepup",
                    serde_json::json!({ "ua": user_agent, "ip": ip }),
                )
                .await?;
            // If Telegram delivery failed we must not lock the user out -
            // fall through to a normal (but alerted) login instead.
            if challenge.delivered {
                let body = OtpConfirmPage {
                    layout: LayoutCtx::anonymous(),
                    title: "Verify it's you".into(),
                    summary: vec![
                        ("Sign-in from".into(), format!("{ip}")),
                        ("Why".into(), "first-seen device or network".into()),
                    ],
                    action_url: "/login/stepup".into(),
                    cancel_url: "/login".into(),
                    action_id: challenge.action_id,
                    demo_otp: None,
                    error: None,
                }
                .render()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("stepup template: {e}")))?;
                return Ok(HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(body));
            }
        }
    }

    session::login(
        &session,
        SessionUser {
            id: user.id,
            email: user.email.clone(),
            name: user.given_name(),
            role: user.role,
        },
    )?;

    // Device tracking: browser family + source IP, with first-seen flags
    // that alert the user and feed the admin's per-user activity view.
    record_login_session(
        &state.db,
        &otp_channel.clone().into_inner(),
        user.id,
        &user_agent,
        &ip,
    )
    .await;

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

    // No auto-login: the user must sign in with their new credentials -
    // verifying the password they just set before any session exists.
    tracing::info!(user_id = user.id, "registration complete; fresh sign-in required");
    Ok(redirect("/login?registered=1"))
}

async fn logout(session: Session) -> impl Responder {
    session::logout(&session);
    redirect("/")
}

// ── Risk-based step-up confirm ───────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StepupForm {
    action_id: i64,
    otp: String,
}

/// Completes a first-seen-origin login: only after the code verifies does a
/// session exist. The pending action itself tells us who is signing in.
async fn login_stepup(
    form: web::Form<StepupForm>,
    svc: web::Data<dyn AuthService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    otp_channel: web::Data<dyn OtpChannel>,
    state: web::Data<AppState>,
    session: Session,
) -> Result<HttpResponse, AppError> {
    // Who does this pending step-up belong to?
    let row: Option<(i64, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT user_id, payload FROM action_otps
        WHERE id = $1 AND purpose = 'login.stepup' AND consumed_at IS NULL
        "#,
    )
    .bind(form.action_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((user_id, payload)) = row else {
        return render_login(
            Some("That verification is no longer valid - please sign in again.".into()),
            None,
            String::new(),
        );
    };

    match otp_svc
        .verify(user_id, form.action_id, "login.stepup", &form.otp)
        .await
    {
        Ok(_) => {}
        // Wrong code: stay on the page and retry the same pending action.
        Err(AppError::BadRequest(msg)) => {
            let body = OtpConfirmPage {
                layout: LayoutCtx::anonymous(),
                title: "Verify it's you".into(),
                summary: vec![],
                action_url: "/login/stepup".into(),
                cancel_url: "/login".into(),
                action_id: form.action_id,
                demo_otp: None,
                error: Some(msg),
            }
            .render()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("stepup template: {e}")))?;
            return Ok(HttpResponse::Ok()
                .content_type("text/html; charset=utf-8")
                .body(body));
        }
        Err(AppError::Conflict(msg)) => {
            // Third strike: the action is cancelled - block this origin from
            // signing in to the account for 24 hours and alert the owner.
            if msg.contains("too many") {
                let ua = payload["ua"].as_str().unwrap_or("unknown");
                let ip = payload["ip"].as_str().unwrap_or("unknown");
                block_origin(
                    &state.db,
                    &otp_channel.clone().into_inner(),
                    user_id,
                    ua,
                    ip,
                    24,
                )
                .await;
                return render_login(
                    Some("Too many invalid codes - this device/network is now blocked from signing in for 24 hours.".into()),
                    None,
                    String::new(),
                );
            }
            return render_login(
                Some("The verification expired - please sign in again.".into()),
                None,
                String::new(),
            );
        }
        Err(other) => return Err(other),
    }

    let user = svc.find_by_id(user_id).await?;
    session::login(
        &session,
        SessionUser {
            id: user.id,
            email: user.email.clone(),
            name: user.given_name(),
            role: user.role,
        },
    )?;

    // Record the (flagged) session - the new-origin alert still fires, which
    // is correct: a step-up SUCCEEDED from a new device, the owner should know.
    let ua = payload["ua"].as_str().unwrap_or("unknown").to_string();
    let ip = payload["ip"].as_str().unwrap_or("unknown").to_string();
    record_login_session(
        &state.db,
        &otp_channel.clone().into_inner(),
        user.id,
        &ua,
        &ip,
    )
    .await;
    let _ = sqlx::query(
        r#"INSERT INTO audit_log (actor_user_id, event, payload) VALUES ($1, 'auth.login.stepup_passed', $2)"#,
    )
    .bind(user.id)
    .bind(serde_json::json!({ "ip": ip }))
    .execute(&state.db)
    .await;

    Ok(redirect(post_login_destination(user.role)))
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
