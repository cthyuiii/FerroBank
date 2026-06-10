//! Loans handlers — owned by the Loans module (Member 5).
//!
//! Routes:
//!   GET  /loans               → customers see their own loans; staff see all loans
//!   GET  /loans/apply         → application form
//!   POST /loans/apply         → submit application
//!   GET  /loans/{id}          → loan detail with repayment history
//!   POST /loans/{id}/repay    → record a repayment (owner only)
//!   POST /loans/{id}/approve  → staff (teller or admin); needs one of each
//!   POST /loans/{id}/reject   → staff (teller or admin)

use actix_web::{web, HttpResponse};
use askama::Template;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::errors::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::account::{Account, AccountStatus};
use crate::models::loan::{Loan, LoanStatus, Repayment};
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::action_otp_service::ActionOtpService;
use crate::services::loan_service::LoanService;
use crate::view::{LayoutCtx, OtpConfirmPage};

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/loans")
            .route("", web::get().to(list))
            .route("/apply", web::get().to(apply_form))
            .route("/apply", web::post().to(apply_submit))
            .route("/apply/confirm", web::post().to(apply_confirm))
            .route("/{id}", web::get().to(detail))
            .route("/{id}/repay", web::post().to(repay))
            .route("/{id}/approve", web::post().to(approve))
            .route("/{id}/reject", web::post().to(reject)),
    );
}

// ── Templates ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "loans/list.html")]
struct ListTemplate {
    layout: LayoutCtx,
    loans: Vec<Loan>,
    /// True when the viewer is a staff member looking at all loans,
    /// false when they're a customer looking at their own loans.
    staff_view: bool,
}

#[derive(Template)]
#[template(path = "loans/apply.html")]
struct ApplyTemplate {
    layout: LayoutCtx,
    error: Option<String>,
    principal: String,
    interest_rate: String,
    term_months: String,
    /// Active accounts the principal can be disbursed to.
    accounts: Vec<Account>,
}

#[derive(Template)]
#[template(path = "loans/detail.html")]
struct DetailTemplate {
    layout: LayoutCtx,
    loan: Loan,
    outstanding: Decimal,
    total_due: Decimal,
    repayments: Vec<Repayment>,
    /// Active accounts the borrower can pay from (only populated for the owner).
    repay_accounts: Vec<Account>,
    can_repay: bool,
    /// Whether THIS staff viewer can still record an approval for their role.
    can_decide: bool,
    /// Dual-approval progress, shown to staff.
    teller_approved: bool,
    admin_approved: bool,
    is_staff: bool,
    /// Inline error (e.g. a failed repayment) shown on the loan page itself.
    error: Option<String>,
}

// ── Form payloads ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ApplyForm {
    principal: String,
    /// Annual rate as a percentage (e.g. "5.25" for 5.25%).
    interest_rate_pct: String,
    term_months: String,
    /// Account credited with the principal when fully approved.
    disbursement_account_id: i64,
}

#[derive(Debug, Deserialize)]
struct RepayForm {
    amount: String,
    account_id: i64,
}

/// Generic OTP confirmation payload (same shape as accounts/settings).
#[derive(Debug, Deserialize)]
struct ActionConfirmForm {
    action_id: i64,
    otp: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn list(
    svc: web::Data<dyn LoanService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Staff see every loan (so they can review approved/active/paid-off ones,
    // not just the pending queue); customers see their own.
    let (loans, staff_view) = match user.role {
        Role::Admin | Role::Teller => (svc.list_all().await?, true),
        Role::Customer => (svc.list_for_user(user.id).await?, false),
    };

    render(ListTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        loans,
        staff_view,
    })
}

async fn apply_form(
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Only customers borrow — staff can't own accounts, so a staff loan could
    // never be repaid (repayments debit a funding account).
    if user.role != Role::Customer {
        return Err(AppError::Forbidden);
    }
    render(ApplyTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        error: None,
        principal: String::new(),
        interest_rate: String::new(),
        term_months: String::new(),
        accounts: active_accounts(&account_svc, user.id).await?,
    })
}

/// The user's active accounts (disbursement / repayment pickers).
async fn active_accounts(
    account_svc: &web::Data<dyn AccountService>,
    user_id: i64,
) -> Result<Vec<Account>, AppError> {
    Ok(account_svc
        .list_for_user(user_id)
        .await?
        .into_iter()
        .filter(|a| a.status == AccountStatus::Active)
        .collect())
}

async fn apply_submit(
    form: web::Form<ApplyForm>,
    otp_svc: web::Data<dyn ActionOtpService>,
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Same guard as the form — POSTs can arrive without visiting the form.
    if user.role != Role::Customer {
        return Err(AppError::Forbidden);
    }
    let form = form.into_inner();
    let accounts = active_accounts(&account_svc, user.id).await?;

    let re_render = |msg: String, accounts: Vec<Account>| -> Result<HttpResponse, AppError> {
        render(ApplyTemplate {
            layout: LayoutCtx::from_user(Some(&user)),
            error: Some(msg),
            principal: form.principal.clone(),
            interest_rate: form.interest_rate_pct.clone(),
            term_months: form.term_months.clone(),
            accounts,
        })
    };

    // The chosen disbursement account must be the applicant's own.
    if !accounts.iter().any(|a| a.id == form.disbursement_account_id) {
        return re_render("Choose one of your active accounts for the payout.".into(), accounts);
    }

    let principal = match Decimal::from_str(form.principal.trim()) {
        Ok(d) => d,
        Err(_) => return re_render("Principal must be a number like 5000.00.".into(), accounts),
    };
    let pct = match Decimal::from_str(form.interest_rate_pct.trim()) {
        Ok(d) => d,
        Err(_) => return re_render("Interest rate must be a number like 5.25.".into(), accounts),
    };
    let interest_rate = pct / Decimal::from(100); // 5.25 → 0.0525
    let term_months = match form.term_months.trim().parse::<i32>() {
        Ok(n) => n,
        Err(_) => return re_render("Term must be a whole number of months.".into(), accounts),
    };

    // Applying for credit is a sensitive action → OTP gate. The application
    // is parked in action_otps and only submitted once the code verifies.
    let challenge = otp_svc
        .begin(
            user.id,
            "loan.apply",
            serde_json::json!({
                "principal": principal.to_string(),
                "interest_rate": interest_rate.to_string(),
                "term_months": term_months,
                "disbursement_account_id": form.disbursement_account_id,
            }),
        )
        .await?;

    let pct = (interest_rate * Decimal::from(100)).round_dp(2).normalize();
    render(OtpConfirmPage {
        layout: LayoutCtx::from_user(Some(&user)),
        title: "Confirm loan application".into(),
        summary: vec![
            ("Principal".into(), format!("${principal}")),
            ("Annual rate".into(), format!("{pct}%")),
            ("Term".into(), format!("{term_months} months")),
        ],
        action_url: "/loans/apply/confirm".into(),
        cancel_url: "/loans".into(),
        action_id: challenge.action_id,
        demo_otp: if challenge.delivered { None } else { Some(challenge.otp) },
        error: None,
    })
}

async fn apply_confirm(
    form: web::Form<ActionConfirmForm>,
    svc: web::Data<dyn LoanService>,
    otp_svc: web::Data<dyn ActionOtpService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    if user.role != Role::Customer {
        return Err(AppError::Forbidden);
    }

    let blank_form = |error: Option<String>, user: &CurrentUser| {
        render(ApplyTemplate {
            layout: LayoutCtx::from_user(Some(user)),
            error,
            principal: String::new(),
            interest_rate: String::new(),
            term_months: String::new(),
            accounts: Vec::new(),
        })
    };

    let payload = match otp_svc
        .verify(user.id, form.action_id, "loan.apply", &form.otp)
        .await
    {
        Ok(p) => p,
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            return blank_form(Some(format!("{msg} — please start again.")), &user);
        }
        Err(other) => return Err(other),
    };

    let principal = Decimal::from_str(payload["principal"].as_str().unwrap_or_default())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("bad loan.apply payload")))?;
    let interest_rate = Decimal::from_str(payload["interest_rate"].as_str().unwrap_or_default())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("bad loan.apply payload")))?;
    let term_months = payload["term_months"].as_i64().unwrap_or_default() as i32;
    let disbursement_account_id = payload["disbursement_account_id"].as_i64().unwrap_or_default();

    match svc
        .apply(user.id, principal, interest_rate, term_months, disbursement_account_id)
        .await
    {
        Ok(loan) => Ok(HttpResponse::Found()
            .insert_header(("Location", format!("/loans/{}", loan.id)))
            .finish()),
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            blank_form(Some(msg), &user)
        }
        Err(other) => Err(other),
    }
}

async fn detail(
    path: web::Path<i64>,
    svc: web::Data<dyn LoanService>,
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let loan_id = path.into_inner();
    render_loan_detail(&svc, &account_svc, &user, loan_id, None).await
}

/// Build the loan detail page. Shared by `GET /loans/{id}` and by `repay` so a
/// failed repayment can re-render the same page with an inline error rather than
/// bouncing to the global error page.
async fn render_loan_detail(
    svc: &web::Data<dyn LoanService>,
    account_svc: &web::Data<dyn AccountService>,
    user: &CurrentUser,
    loan_id: i64,
    error: Option<String>,
) -> Result<HttpResponse, AppError> {
    let loan = svc.get_by_id(loan_id).await?;

    // Customer can only see their own; staff can see any.
    if loan.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    let outstanding = svc.outstanding_balance(loan_id).await?;
    let repayments = svc.list_repayments(loan_id).await?;

    let months_factor = Decimal::from(loan.term_months) / Decimal::from(12);
    let total_due =
        (loan.principal + loan.principal * loan.interest_rate * months_factor).round_dp(2);

    let can_repay = (loan.status == LoanStatus::Approved || loan.status == LoanStatus::Active)
        && loan.user_id == user.id;

    // Funding accounts for the repay form — only the borrower needs them.
    let repay_accounts: Vec<Account> = if can_repay {
        account_svc
            .list_for_user(user.id)
            .await?
            .into_iter()
            .filter(|a| a.status == AccountStatus::Active)
            .collect()
    } else {
        Vec::new()
    };

    // Dual-approval state.
    let (teller_approved, admin_approved) = svc.approval_flags(loan_id).await?;
    let is_staff = user.role != Role::Customer;
    let already_approved_this_role = match user.role {
        Role::Admin => admin_approved,
        Role::Teller => teller_approved,
        Role::Customer => true, // customers never decide
    };
    let can_decide =
        is_staff && loan.status == LoanStatus::Pending && !already_approved_this_role;

    render(DetailTemplate {
        layout: LayoutCtx::from_user(Some(user)),
        loan,
        outstanding,
        total_due,
        repayments,
        repay_accounts,
        can_repay,
        can_decide,
        teller_approved,
        admin_approved,
        is_staff,
        error,
    })
}

async fn repay(
    path: web::Path<i64>,
    form: web::Form<RepayForm>,
    svc: web::Data<dyn LoanService>,
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let loan_id = path.into_inner();

    // Owner-only.
    let loan = svc.get_by_id(loan_id).await?;
    if loan.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    // The funding account must belong to the borrower.
    let account = account_svc.get_by_id(form.account_id).await?;
    if account.user_id != user.id {
        return Err(AppError::Forbidden);
    }

    let amount = match Decimal::from_str(form.amount.trim()) {
        Ok(d) => d,
        Err(_) => {
            return render_loan_detail(
                &svc,
                &account_svc,
                &user,
                loan_id,
                Some("Repayment must be a number like 100.00.".into()),
            )
            .await;
        }
    };

    match svc.record_repayment(loan_id, amount, form.account_id).await {
        // Success → back to the loan page.
        Ok(_) => Ok(HttpResponse::Found()
            .insert_header(("Location", format!("/loans/{loan_id}")))
            .finish()),
        // Business rejections (insufficient funds, frozen account, etc.) show
        // inline on the loan page instead of the 409 error page.
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            render_loan_detail(&svc, &account_svc, &user, loan_id, Some(msg)).await
        }
        // A genuine mid-transaction/internal failure still surfaces as an error.
        Err(other) => Err(other),
    }
}

async fn approve(
    path: web::Path<i64>,
    svc: web::Data<dyn LoanService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Both tellers and admins may record an approval; the service enforces that
    // one of each (two distinct people) is required before the loan is approved.
    if user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }
    let loan_id = path.into_inner();
    svc.approve(loan_id, user.id, user.role).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", format!("/loans/{loan_id}")))
        .finish())
}

async fn reject(
    path: web::Path<i64>,
    svc: web::Data<dyn LoanService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    // Either staff role can reject a pending application.
    if user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }
    let loan_id = path.into_inner();
    svc.reject(loan_id).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", format!("/loans/{loan_id}")))
        .finish())
}

// ── Render helper ────────────────────────────────────────────────────

fn render<T: Template>(tmpl: T) -> Result<HttpResponse, AppError> {
    let body = tmpl
        .render()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("loans template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
