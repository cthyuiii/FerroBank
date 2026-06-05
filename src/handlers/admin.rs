//! Admin handlers — owned by the Platform Lead (Member 1).
//!
//! Aggregates read-only data from every module into a single dashboard, plus
//! the staff-facing management screens (all accounts / all transfers) and the
//! account CRUD actions. The whole scope is protected by `RequireRole(Admin)`.

use actix_web::{web, HttpResponse};
use askama::Template;
use chrono::NaiveDate;
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
use crate::state::AppState;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    // Admin-only: the dashboard, the audit log, and the account mutations
    // (open / freeze / unfreeze / close / adjust).
    cfg.service(
        web::scope("/admin")
            .wrap(RequireRole(Role::Admin))
            .route("/dashboard", web::get().to(dashboard))
            .route("/audit", web::get().to(audit))
            .route("/accounts/open", web::post().to(account_open))
            .route("/accounts/{id}/freeze", web::post().to(account_freeze))
            .route("/accounts/{id}/unfreeze", web::post().to(account_unfreeze))
            .route("/accounts/{id}/close", web::post().to(account_close))
            .route("/accounts/{id}/adjust", web::post().to(account_adjust)),
    );

    // Staff area — tellers AND admins (RequireRole(Teller) treats admin as a
    // superuser). Tellers manage account approvals and view all transfers here.
    cfg.service(
        web::scope("/staff")
            .wrap(RequireRole(Role::Teller))
            .route("/accounts", web::get().to(accounts))
            .route("/accounts/{id}/approve", web::post().to(account_approve))
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
    /// Admins get the full CRUD controls; tellers only get "approve".
    is_admin: bool,
}

async fn accounts(
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let accounts = svc.all_accounts().await?;
    // Only customers may own accounts, so the "open for a user" picker lists
    // customers only (no admin/teller staff).
    let users = svc.customers().await?;
    let is_admin = user.role == Role::Admin;
    render(AccountsTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        accounts,
        users,
        is_admin,
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
    state: web::Data<AppState>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let kind = match form.kind.as_str() {
        "savings" => AccountType::Savings,
        "checking" => AccountType::Checking,
        _ => return Err(AppError::BadRequest("choose savings or checking".into())),
    };

    // Accounts may only be opened for customers. The database enforces this too
    // (trigger in 002_accounts.sql), but check here for a friendly error.
    let target_role: Role = sqlx::query_scalar(r#"SELECT role FROM users WHERE id = $1"#)
        .bind(form.user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("that user does not exist".into()))?;
    if target_role != Role::Customer {
        return Err(AppError::BadRequest(
            "accounts can only be opened for customers".into(),
        ));
    }

    // Admin opening on a customer's behalf is approved immediately.
    let account = account_svc.open_account(form.user_id, kind, true).await?;
    audit_svc
        .record(
            Some(user.id),
            "admin.account.opened",
            json!({ "account_id": account.id, "for_user": form.user_id, "kind": form.kind }),
        )
        .await?;
    Ok(redirect("/staff/accounts"))
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
    Ok(redirect("/staff/accounts"))
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
    Ok(redirect("/staff/accounts"))
}

/// Staff (teller or admin) approve a pending account, making it active.
async fn account_approve(
    path: web::Path<i64>,
    account_svc: web::Data<dyn AccountService>,
    audit_svc: web::Data<dyn AuditService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let id = path.into_inner();
    account_svc.approve_account(id).await?;
    audit_svc
        .record(Some(user.id), "account.approved", json!({ "account_id": id }))
        .await?;
    Ok(redirect("/staff/accounts"))
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
    Ok(redirect("/staff/accounts"))
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
    Ok(redirect("/staff/accounts"))
}

// ── All transfers ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/transfers.html")]
struct TransfersTemplate {
    layout: LayoutCtx,
    transfers: Vec<AdminTransferRow>,
    /// Echoed back into the date inputs so the chosen range sticks.
    from_date: String,
    to_date: String,
    /// Admins see a dashboard link; tellers don't.
    is_admin: bool,
}

#[derive(Debug, Deserialize)]
struct TransferFilter {
    from: Option<String>,
    to: Option<String>,
}

async fn transfers(
    query: web::Query<TransferFilter>,
    svc: web::Data<dyn AdminService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Parse the optional date bounds from the query string (HTML date inputs
    // submit `YYYY-MM-DD`); ignore anything unparseable.
    fn parse(s: &Option<String>) -> Option<NaiveDate> {
        s.as_deref()
            .filter(|v| !v.is_empty())
            .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
    }
    let from = parse(&query.from);
    let to = parse(&query.to);

    let transfers = svc.all_transfers(from, to).await?;
    let is_admin = user.role == Role::Admin;
    render(TransfersTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        transfers,
        from_date: query.from.clone().unwrap_or_default(),
        to_date: query.to.clone().unwrap_or_default(),
        is_admin,
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
