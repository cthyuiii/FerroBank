//! Accounts handlers — owned by the Accounts module (Member 3).
//!
//! Flow:
//!   GET  /accounts            → list current user's accounts
//!   GET  /accounts/new        → form to choose account type
//!   POST /accounts/new        → open a new account, redirect to detail
//!   GET  /accounts/:id        → show account detail
//!   POST /accounts/:id/freeze → staff-only, freeze the account
//!   POST /accounts/:id/close  → owner or staff, close a zero-balance account

use actix_web::{web, HttpResponse};
use askama::Template;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::errors::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::account::{Account, AccountStatus, AccountType};
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/accounts")
            .route("", web::get().to(list))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/{id}", web::get().to(detail))
            .route("/{id}/freeze", web::post().to(freeze))
            .route("/{id}/close", web::post().to(close)),
    );
}

// ── Templates ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "accounts/list.html")]
struct ListTemplate {
    layout: LayoutCtx,
    accounts: Vec<Account>,
    total_balance: Decimal,
}

#[derive(Template)]
#[template(path = "accounts/new.html")]
struct NewTemplate {
    layout: LayoutCtx,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "accounts/detail.html")]
struct DetailTemplate {
    layout: LayoutCtx,
    account: Account,
    can_manage: bool,
}

// ── Form payloads ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct NewAccountForm {
    kind: String, // "savings" | "checking"
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn list(
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let accounts = svc.list_for_user(user.id).await?;
    let total_balance: Decimal = accounts
        .iter()
        .filter(|a| a.status == AccountStatus::Active)
        .map(|a| a.balance)
        .sum();

    render(ListTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        accounts,
        total_balance,
    })
}

async fn new_form(user: CurrentUser) -> Result<HttpResponse, AppError> {
    render(NewTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        error: None,
    })
}

async fn create(
    form: web::Form<NewAccountForm>,
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let kind = match form.kind.as_str() {
        "savings" => AccountType::Savings,
        "checking" => AccountType::Checking,
        _ => {
            return render(NewTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                error: Some("Please choose Savings or Checking.".into()),
            });
        }
    };

    let account = svc.open_account(user.id, kind).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", format!("/accounts/{}", account.id)))
        .finish())
}

async fn detail(
    path: web::Path<i64>,
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let account_id = path.into_inner();
    let account = svc.get_by_id(account_id).await?;

    // Customers can only see their own accounts. Staff can see any.
    if account.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    // Owners always see manage buttons; staff also see them on any account.
    let can_manage = account.user_id == user.id || user.role != Role::Customer;

    render(DetailTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        account,
        can_manage,
    })
}

async fn freeze(
    path: web::Path<i64>,
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Only staff (Teller / Admin) can freeze accounts.
    if user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    let account_id = path.into_inner();
    svc.freeze_account(account_id).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", format!("/accounts/{account_id}")))
        .finish())
}

async fn close(
    path: web::Path<i64>,
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let account_id = path.into_inner();

    // Owner or staff can close.
    let account = svc.get_by_id(account_id).await?;
    if account.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    svc.close_account(account_id).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", "/accounts"))
        .finish())
}

// ── Render helper ────────────────────────────────────────────────────

fn render<T: Template>(tmpl: T) -> Result<HttpResponse, AppError> {
    let body = tmpl
        .render()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("accounts template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
