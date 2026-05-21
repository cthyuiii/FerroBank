//! Loans handlers — owned by the Loans module (Member 5).
//!
//! Admin-only endpoints (approve / reject) should be wrapped with
//! `RequireRole(Role::Admin)` at the route level.
//!
//! Use `web::Data<dyn LoanService>` for dependency injection.

use actix_web::{web, Responder};

use super::coming_soon;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/loans")
            .route("", web::get().to(list))
            .route("/apply", web::get().to(apply_form))
            .route("/apply", web::post().to(apply_submit))
            .route("/{id}", web::get().to(detail))
            .route("/{id}/repay", web::post().to(repay)),
    );
}

async fn list() -> impl Responder {
    coming_soon("Loans · List", "Member 5")
}

async fn apply_form() -> impl Responder {
    coming_soon("Loans · Apply", "Member 5")
}

async fn apply_submit() -> impl Responder {
    coming_soon("Loans · Apply submit", "Member 5")
}

async fn detail(_path: web::Path<i64>) -> impl Responder {
    coming_soon("Loans · Detail", "Member 5")
}

async fn repay(_path: web::Path<i64>) -> impl Responder {
    coming_soon("Loans · Repay", "Member 5")
}
