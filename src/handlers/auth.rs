//! Auth handlers — owned by the Auth module (Member 2).
//!
//! When you (Member 2) implement these:
//!   - Use `web::Form<RegisterForm>` and `web::Form<LoginForm>` extractors.
//!   - Validate the form with the `validator` crate.
//!   - Call `auth.register(...)` / `auth.login(...)` on the injected service.
//!   - On successful login, call `crate::middleware::auth::session::login(&session, SessionUser { ... })`.
//!   - On logout, call `crate::middleware::auth::session::logout(&session)`.
//!   - Render templates from `templates/auth/`.

use actix_web::{web, HttpResponse, Responder};

use super::coming_soon;

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

async fn login_form() -> impl Responder {
    coming_soon("Auth · Login form", "Member 2")
}

async fn login_submit() -> impl Responder {
    coming_soon("Auth · Login submit", "Member 2")
}

async fn register_form() -> impl Responder {
    coming_soon("Auth · Register form", "Member 2")
}

async fn register_submit() -> impl Responder {
    coming_soon("Auth · Register submit", "Member 2")
}

async fn logout() -> impl Responder {
    // Even the stub does the right thing: nuke the session cookie and bounce.
    HttpResponse::Found()
        .insert_header(("Location", "/login"))
        .finish()
}
