//! Public landing pages — owned by the Platform Lead (Member 1).

use actix_web::{web, HttpResponse};
use askama::Template;

use crate::errors::AppError;
use crate::middleware::auth::CurrentUser;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(home));
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    layout: LayoutCtx,
}

// `Option<CurrentUser>` resolves to `None` for anonymous visitors and to the
// logged-in user otherwise, so the landing page reflects session state in the
// nav instead of always rendering the signed-out view.
async fn home(user: Option<CurrentUser>) -> Result<HttpResponse, AppError> {
    let body = HomeTemplate {
        layout: LayoutCtx::from_user(user.as_ref()),
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("home template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
