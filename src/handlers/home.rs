//! Public landing pages — owned by the Platform Lead (Member 1).

use actix_web::{web, HttpResponse};
use askama::Template;

use crate::errors::AppError;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(home));
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    layout: LayoutCtx,
}

async fn home() -> Result<HttpResponse, AppError> {
    let body = HomeTemplate {
        layout: LayoutCtx::anonymous(),
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("home template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
