//! Transfers handlers — owned by the Transfers module (Member 4).
//!
//! Two-step flow:
//!   POST /transfers/new      → svc.create(...)  → renders confirm.html with OTP field
//!   POST /transfers/confirm  → svc.confirm(...) → money actually moves
//!
//! Use `web::Data<dyn TransferService>` for dependency injection.

use actix_web::{web, Responder};

use super::coming_soon;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/transfers")
            .route("", web::get().to(history))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/confirm", web::post().to(confirm)),
    );
}

async fn history() -> impl Responder {
    coming_soon("Transfers · History", "Member 4")
}

async fn new_form() -> impl Responder {
    coming_soon("Transfers · New", "Member 4")
}

async fn create() -> impl Responder {
    coming_soon("Transfers · Create", "Member 4")
}

async fn confirm() -> impl Responder {
    coming_soon("Transfers · Confirm", "Member 4")
}
