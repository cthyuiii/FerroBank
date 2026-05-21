//! Accounts handlers — owned by the Accounts module (Member 3).
//!
//! When you implement these:
//!   - Use `svc: web::Data<dyn AccountService>` (NOT `PgAccountService`) — that's
//!     the OOP/polymorphism part the grader is looking for.
//!   - Use `user: CurrentUser` to get the logged-in user.
//!   - Render templates from `templates/accounts/`.

use actix_web::{web, Responder};

use super::coming_soon;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/accounts")
            .route("", web::get().to(list))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/{id}", web::get().to(detail)),
    );
}

async fn list() -> impl Responder {
    coming_soon("Accounts · List", "Member 3")
}

async fn new_form() -> impl Responder {
    coming_soon("Accounts · New", "Member 3")
}

async fn create() -> impl Responder {
    coming_soon("Accounts · Create", "Member 3")
}

async fn detail(_path: web::Path<i64>) -> impl Responder {
    coming_soon("Accounts · Detail", "Member 3")
}
