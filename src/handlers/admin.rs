//! Admin handlers — owned by the Platform Lead (Member 1).
//!
//! Aggregates read-only data from every module into a single dashboard, plus
//! the staff-facing management screens (all accounts / all transfers) and the
//! account CRUD actions. The whole scope is protected by `RequireRole(Admin)`.

use actix_web::{web, HttpResponse};
use askama::Template;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;

use crate::errors::AppError;
use crate::middleware::auth::{CurrentUser, RequireRole};
use crate::models::account::AccountType;
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::admin_service::{
    AdminAccountRow, AdminService, AdminTransferRow, AdminUserRow, DashboardSnapshot,
};
use crate::services::audit_service::{AuditEntry, AuditService};
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/admin")
            .wrap(RequireRole(Role::Admin))
            .route("/dashboard", web::get().to(dashboard))
            .route("/audit", web::get().to(audit))
            .route("/accounts", web::get().to(accounts))
            .route("/accounts/open", web::post().to(account_open))
            .route("/accounts/{id}/freeze", web::post().to(account_freeze))
            .route("/accounts/{id}/unfreeze", web::post().to(account_unfreeze))
            .route("/accounts/{id}/close", web::post().to(account_close))
            .route("/accounts/{id}/adjust", web::post().to(account_adjust))
            .route("/transfers", web::get().to(transfers)),
    );
}

// ── Dashboard ────────────────────────────────────────────────────────

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
    render(DashboardTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        snapshot,
    })
}

// ── Audit log ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/audit.html")]
struct AuditTemplate {
    layout: LayoutCtx,
    entries: Vec<AuditEntry>,
}

async fn audit(
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Pull a generous window so the client-side search has something to filter.
    let entries = audit_svc.recent(500).await?;
    render(AuditTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        entries,
    })
}

// ── All accounts (with CRUD) ─────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/accounts.html")]
struct AccountsTemplate {
    layout: LayoutCtx,
    accounts: Vec<AdminAccountRow>,
    users: Vec<AdminUserRow>,
}

async fn accounts(
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let accounts = svc.all_accounts().await?;
    let users = svc.all_users().await?;
    render(AccountsTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        accounts,
        users,
    })
}

#[derive(Debug, Deserialize)]
struct OpenAccountForm {
    user_id: i64,
    kind: String,
}

async fn account_open(
    form: web::Form<OpenAccountForm>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let kind = match form.kind.as_str() {
        "savings" => AccountType::Savings,
        "checking" => AccountType::Checking,
        _ => return Err(AppError::BadRequest("choose savings or checking".into())),
    };
    let account = account_svc.open_account(form.user_id, kind).await?;
    audit_svc
        .record(
            Some(user.id),
            "admin.account.opened",
            json!({ "account_id": account.id, "for_user": form.user_id, "kind": form.kind }),
        )
        .await?;
    Ok(redirect("/admin/accounts"))
}

async fn account_freeze(
    path: web::Path<i64>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let id = path.into_inner();
    account_svc.freeze_account(id).await?;
    audit_svc
        .record(Some(user.id), "admin.account.frozen", json!({ "account_id": id }))
        .await?;
    Ok(redirect("/admin/accounts"))
}

async fn account_unfreeze(
    path: web::Path<i64>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let id = path.into_inner();
    account_svc.unfreeze_account(id).await?;
    audit_svc
        .record(Some(user.id), "admin.account.unfrozen", json!({ "account_id": id }))
        .await?;
    Ok(redirect("/admin/accounts"))
}

async fn account_close(
    path: web::Path<i64>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let id = path.into_inner();
    account_svc.close_account(id).await?;
    audit_svc
        .record(Some(user.id), "admin.account.closed", json!({ "account_id": id }))
        .await?;
    Ok(redirect("/admin/accounts"))
}

#[derive(Debug, Deserialize)]
struct AdjustForm {
    /// Signed amount: positive credits the account, negative debits it.
    delta: String,
}

async fn account_adjust(
    path: web::Path<i64>,
    form: web::Form<AdjustForm>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let id = path.into_inner();
    let delta = Decimal::from_str(form.delta.trim()).map_err(|_| {
        AppError::BadRequest("adjustment must be a number like 50.00 or -50.00".into())
    })?;
    let new_balance = account_svc.adjust_balance(id, delta).await?;
    audit_svc
        .record(
            Some(user.id),
            "admin.account.adjusted",
            json!({
                "account_id": id,
                "delta": delta.to_string(),
                "new_balance": new_balance.to_string()
            }),
        )
        .await?;
    Ok(redirect("/admin/accounts"))
}

// ── All transfers ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/transfers.html")]
struct TransfersTemplate {
    layout: LayoutCtx,
    transfers: Vec<AdminTransferRow>,
}

async fn transfers(
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let transfers = svc.all_transfers().await?;
    render(TransfersTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        transfers,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

fn redirect(location: &str) -> HttpResponse {
    HttpResponse::Found()
        .insert_header(("Location", location))
        .finish()
}

fn render<T: Template>(tmpl: T) -> Result<HttpResponse, AppError> {
    let body = tmpl
        .render()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("admin template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
