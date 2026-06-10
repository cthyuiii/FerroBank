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
use crate::models::loan::LoanStatus;
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::action_otp_service::ActionOtpService;
use crate::services::loan_service::LoanService;
use crate::view::{LayoutCtx, OtpConfirmPage};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/accounts")
            .route("", web::get().to(list))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/new/confirm", web::post().to(create_confirm))
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
    /// Sum of active account balances.
    total_balance: Decimal,
    /// Sum of outstanding balances across the user's active/approved loans.
    total_outstanding: Decimal,
    /// total_balance − total_outstanding.
    net_worth: Decimal,
    /// True when net worth is zero or positive (green) vs negative (red).
    net_worth_positive: bool,
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
    /// Staff (teller/admin) — only they may freeze an account.
    is_staff: bool,
}

// ── Form payloads ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct NewAccountForm {
    kind: String, // "savings" | "checking"
}

/// Generic OTP confirmation payload (shared shape with loans/settings).
#[derive(Debug, Deserialize)]
struct ActionConfirmForm {
    action_id: i64,
    otp: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn list(
    svc: web::Data<dyn AccountService>,
    loan_svc: web::Data<dyn LoanService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let accounts = svc.list_for_user(user.id).await?;
    let total_balance: Decimal = accounts
        .iter()
        .filter(|a| a.status == AccountStatus::Active)
        .map(|a| a.balance)
        .sum();

    // Sum what the user still owes across their live loans.
    let loans = loan_svc.list_for_user(user.id).await?;
    let mut total_outstanding = Decimal::ZERO;
    for l in &loans {
        if l.status == LoanStatus::Approved || l.status == LoanStatus::Active {
            total_outstanding += loan_svc.outstanding_balance(l.id).await?;
        }
    }
    let net_worth = total_balance - total_outstanding;
    let net_worth_positive = net_worth >= Decimal::ZERO;

    render(ListTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        accounts,
        total_balance,
        total_outstanding,
        net_worth,
        net_worth_positive,
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
    otp_svc: web::Data<dyn ActionOtpService>,
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

    // Opening an account is a sensitive action → OTP gate. Nothing is created
    // until the code verifies; the request is parked in action_otps.
    let challenge = otp_svc
        .begin(user.id, "account.open", serde_json::json!({ "kind": form.kind }))
        .await?;

    render(OtpConfirmPage {
        layout: LayoutCtx::from_user(Some(&user)),
        title: "Confirm new account".into(),
        summary: vec![("Account type".into(), kind.label().to_string())],
        action_url: "/accounts/new/confirm".into(),
        cancel_url: "/accounts".into(),
        action_id: challenge.action_id,
        demo_otp: if challenge.delivered { None } else { Some(challenge.otp) },
        error: None,
    })
}

async fn create_confirm(
    form: web::Form<ActionConfirmForm>,
    svc: web::Data<dyn AccountService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let payload = match otp_svc
        .verify(user.id, form.action_id, "account.open", &form.otp)
        .await
    {
        Ok(p) => p,
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            return render(NewTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                error: Some(format!("{msg} — please start again.")),
            });
        }
        Err(other) => return Err(other),
    };

    let kind = match payload["kind"].as_str() {
        Some("savings") => AccountType::Savings,
        Some("checking") => AccountType::Checking,
        _ => return Err(AppError::Internal(anyhow::anyhow!("bad account.open payload"))),
    };

    // Customer self-opens are created pending — a teller/admin must approve.
    let account = svc.open_account(user.id, kind, false).await?;

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
    // Only staff may freeze an account — normal users must not see that option.
    let is_staff = user.role != Role::Customer;

    render(DetailTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        account,
        can_manage,
        is_staff,
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
