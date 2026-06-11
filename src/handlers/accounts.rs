//! Accounts handlers - owned by the Accounts module (Member 3).
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
use std::str::FromStr;

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
            .route("/{id}/close", web::post().to(close))
            .route("/{id}/limit", web::post().to(limit_change))
            .route("/{id}/limit/confirm", web::post().to(limit_change_confirm))
            .route("/{id}/status", web::get().to(status)),
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
    /// Staff (teller/admin) - only they may freeze an account.
    is_staff: bool,
    /// A limit increase waiting out its hold window: (new limit, effective at).
    pending_limit: Option<(Decimal, chrono::DateTime<chrono::Utc>)>,
    /// Inline outcome message for a just-submitted limit request.
    limit_msg: Option<String>,
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
        // Wrong code: the action is still pending - retry inline.
        Err(AppError::BadRequest(msg)) => {
            return render(OtpConfirmPage {
                layout: LayoutCtx::from_user(Some(&user)),
                title: "Confirm new account".into(),
                summary: vec![],
                action_url: "/accounts/new/confirm".into(),
                cancel_url: "/accounts".into(),
                action_id: form.action_id,
                demo_otp: None,
                error: Some(msg),
            });
        }
        Err(AppError::Conflict(msg)) => {
            return render(NewTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                error: Some(format!("{msg} - please start again.")),
            });
        }
        Err(other) => return Err(other),
    };

    let kind = match payload["kind"].as_str() {
        Some("savings") => AccountType::Savings,
        Some("checking") => AccountType::Checking,
        _ => return Err(AppError::Internal(anyhow::anyhow!("bad account.open payload"))),
    };

    // Customer self-opens are created pending - a teller/admin must approve.
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
    render_detail(path.into_inner(), &svc, &user, None).await
}

/// Shared by `detail` and `limit_change` so the limit form can show its
/// outcome inline on the same page.
async fn render_detail(
    account_id: i64,
    svc: &web::Data<dyn AccountService>,
    user: &CurrentUser,
    limit_msg: Option<String>,
) -> Result<HttpResponse, AppError> {
    let account = svc.get_by_id(account_id).await?;

    // Customers can only see their own accounts. Staff can see any.
    if account.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    // Owners always see manage buttons; staff also see them on any account.
    let can_manage = account.user_id == user.id || user.role != Role::Customer;
    // Only staff may freeze an account - normal users must not see that option.
    let is_staff = user.role != Role::Customer;
    let pending_limit = svc.pending_limit_change(account_id).await?;

    render(DetailTemplate {
        layout: LayoutCtx::from_user(Some(user)),
        account,
        can_manage,
        is_staff,
        pending_limit,
        limit_msg,
    })
}

#[derive(Debug, Deserialize)]
struct LimitForm {
    new_limit: String,
}

/// Owner requests a transfer-limit change. The page's consent popup has
/// already warned that increases are held (12 h, or this account's window)
/// before taking effect.
async fn limit_change(
    path: web::Path<i64>,
    form: web::Form<LimitForm>,
    svc: web::Data<dyn AccountService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let account_id = path.into_inner();
    let account = svc.get_by_id(account_id).await?;
    if account.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    let new_limit = match Decimal::from_str(form.new_limit.trim()) {
        Ok(d) if d > Decimal::ZERO => d,
        _ => {
            return render_detail(
                account_id,
                &svc,
                &user,
                Some("The limit must be a positive number like 8000.00.".into()),
            )
            .await
        }
    };

    // Changing a limit is a sensitive action: nothing is posted until the
    // one-time code verifies.
    let challenge = otp_svc
        .begin(
            user.id,
            "limit.change",
            serde_json::json!({ "account_id": account_id, "new_limit": new_limit.to_string() }),
        )
        .await?;

    render(OtpConfirmPage {
        layout: LayoutCtx::from_user(Some(&user)),
        title: "Confirm limit change".into(),
        summary: vec![
            ("Account".into(), account.account_number.clone()),
            ("Current limit".into(), format!("${}", account.transfer_limit)),
            ("Requested limit".into(), format!("${new_limit}")),
        ],
        action_url: format!("/accounts/{account_id}/limit/confirm"),
        cancel_url: format!("/accounts/{account_id}"),
        action_id: challenge.action_id,
        demo_otp: if challenge.delivered { None } else { Some(challenge.otp) },
        error: None,
    })
}

async fn limit_change_confirm(
    path: web::Path<i64>,
    form: web::Form<ActionConfirmForm>,
    svc: web::Data<dyn AccountService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let account_id = path.into_inner();
    let account = svc.get_by_id(account_id).await?;
    if account.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    let payload = match otp_svc
        .verify(user.id, form.action_id, "limit.change", &form.otp)
        .await
    {
        Ok(p) => p,
        // Wrong code: retry inline on the same confirmation page.
        Err(AppError::BadRequest(msg)) => {
            return render(OtpConfirmPage {
                layout: LayoutCtx::from_user(Some(&user)),
                title: "Confirm limit change".into(),
                summary: vec![],
                action_url: format!("/accounts/{account_id}/limit/confirm"),
                cancel_url: format!("/accounts/{account_id}"),
                action_id: form.action_id,
                demo_otp: None,
                error: Some(msg),
            });
        }
        Err(AppError::Conflict(msg)) => {
            return render_detail(account_id, &svc, &user, Some(format!("{msg} - please start again."))).await;
        }
        Err(other) => return Err(other),
    };
    if payload["account_id"].as_i64() != Some(account_id) {
        return Err(AppError::BadRequest("this confirmation belongs to a different account".into()));
    }
    let new_limit = Decimal::from_str(payload["new_limit"].as_str().unwrap_or_default())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("bad limit.change payload")))?;

    match svc.request_limit_change(account_id, new_limit).await {
        Ok(_) => {
            let msg = if new_limit <= account.transfer_limit {
                format!("Limit lowered to ${new_limit}, effective immediately.")
            } else {
                format!("Limit increase to ${new_limit} accepted - it takes effect at the time shown above (your local time).")
            };
            render_detail(account_id, &svc, &user, Some(msg)).await
        }
        Err(AppError::BadRequest(m)) | Err(AppError::Conflict(m)) => {
            render_detail(account_id, &svc, &user, Some(m)).await
        }
        Err(other) => Err(other),
    }
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
    match svc.freeze_account(account_id).await {
        // Already frozen (double submit): the page shows the state.
        Ok(()) | Err(AppError::Conflict(_)) => {}
        Err(e) => return Err(e),
    }

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

    match svc.close_account(account_id).await {
        Ok(()) => Ok(HttpResponse::Found()
            .insert_header(("Location", "/accounts"))
            .finish()),
        // Not closable (already closed, or still holds money): show why
        // inline on the account page instead of a 409.
        Err(AppError::Conflict(msg)) => {
            render_detail(account_id, &svc, &user, Some(msg)).await
        }
        Err(e) => Err(e),
    }
}

/// Live status for pending accounts: the page refreshes itself the moment a
/// teller approves (or the account otherwise changes state).
async fn status(
    path: web::Path<i64>,
    svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let account = svc.get_by_id(path.into_inner()).await?;
    if account.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }
    Ok(HttpResponse::Ok().json(serde_json::json!({ "status": account.status.label() })))
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
