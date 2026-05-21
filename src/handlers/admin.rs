//! Admin handlers — owned by the Platform Lead (Member 1).
//!
//! Aggregates read-only data from every module into a single dashboard.
//! Protected by `RequireRole(Role::Admin)`.

use actix_web::{web, HttpResponse};
use askama::Template;

use crate::errors::AppError;
use crate::middleware::auth::{CurrentUser, RequireRole};
use crate::models::user::Role;
use crate::services::admin_service::{AdminService, DashboardSnapshot};
use crate::services::audit_service::AuditEntry;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/admin")
            .wrap(RequireRole(Role::Admin))
            .route("/dashboard", web::get().to(dashboard))
            .route("/audit", web::get().to(audit)),
    );
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardTemplate {
    layout: LayoutCtx,
    snapshot: DashboardSnapshot,
}

async fn dashboard(
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let snapshot = svc.snapshot().await?;

    let body = DashboardTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        snapshot,
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("dashboard template: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}

#[derive(Template)]
#[template(path = "admin/audit.html")]
struct AuditTemplate {
    layout: LayoutCtx,
    entries: Vec<AuditEntry>,
}

async fn audit(
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let snapshot = svc.snapshot().await?;

    let body = AuditTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        entries: snapshot.recent_audit,
    }
    .render()
    .map_err(|e| AppError::Internal(anyhow::anyhow!("audit template: {e}")))?;

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
