//! Transfers handlers — owned by the Transfers module (Member 4).
//!
//! Two-step flow:
//!   GET  /transfers           → user's transfer history
//!   GET  /transfers/new       → form (accepts ?from=<id> to pre-select)
//!   POST /transfers/new       → create pending row + render confirm page (with OTP)
//!   POST /transfers/confirm   → verify OTP, move money inside one SQL txn, redirect
//!
//! The OTP is displayed on screen for the demo; in production it would be sent
//! by SMS via something like Twilio. See ARCHITECTURE.md.

use actix_web::{web, HttpResponse};
use askama::Template;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::errors::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::account::{Account, AccountStatus};
use crate::models::transfer::Transfer;
// TransferStatus is referenced inside `templates/transfers/history.html`.
// Askama's derive expansion uses it, but the unused-imports lint runs before
// macro expansion and would otherwise warn.
#[allow(unused_imports)]
use crate::models::transfer::TransferStatus;
use crate::models::user::Role;
use crate::services::account_service::AccountService;
use crate::services::transfer_service::TransferService;
use crate::state::AppState;
use crate::view::LayoutCtx;

pub fn routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/transfers")
            .route("", web::get().to(history))
            .route("/new", web::get().to(new_form))
            .route("/new", web::post().to(create))
            .route("/confirm", web::post().to(confirm)),
    );
}

// ── Templates ────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "transfers/new.html")]
struct NewTemplate {
    layout: LayoutCtx,
    /// One option per active account, with `selected` pre-computed in the handler
    /// so the template doesn't need to dereference an Option (Askama's expression
    /// parser doesn't accept the unary `*` operator).
    accounts: Vec<AccountOption>,
    error: Option<String>,
    /// Sticky form values so the user doesn't have to retype after a validation miss.
    to_account_number: String,
    amount: String,
    note: String,
}

/// Render-side wrapper for one item in the from-account dropdown.
struct AccountOption {
    account: Account,
    selected: bool,
}

#[derive(Template)]
#[template(path = "transfers/confirm.html")]
struct ConfirmTemplate {
    layout: LayoutCtx,
    transfer: Transfer,
    /// Recipient account number (as typed) and resolved owner name, so the
    /// sender can verify who they're paying before confirming.
    to_account_number: String,
    to_owner_name: String,
    /// Demo only. Real system would send this by SMS.
    demo_otp: String,
}

#[derive(Template)]
#[template(path = "transfers/history.html")]
struct HistoryTemplate {
    layout: LayoutCtx,
    transfers: Vec<Transfer>,
    /// Account IDs the current user owns — used to flag in/out per row.
    owned_account_ids: std::collections::HashSet<i64>,
}

// ── Form payloads ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct NewQuery {
    from: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct NewTransferForm {
    from_account_id: i64,
    to_account_number: String,
    amount: String,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfirmForm {
    transfer_id: i64,
    otp: String,
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn history(
    svc: web::Data<dyn TransferService>,
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let transfers = svc.history(user.id).await?;

    let owned_account_ids: std::collections::HashSet<i64> = account_svc
        .list_for_user(user.id)
        .await?
        .into_iter()
        .map(|a| a.id)
        .collect();

    render(HistoryTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        transfers,
        owned_account_ids,
    })
}

async fn new_form(
    query: web::Query<NewQuery>,
    account_svc: web::Data<dyn AccountService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let selected = query.from;
    let accounts: Vec<AccountOption> = account_svc
        .list_for_user(user.id)
        .await?
        .into_iter()
        .filter(|a| a.status == AccountStatus::Active)
        .map(|a| AccountOption {
            selected: selected == Some(a.id),
            account: a,
        })
        .collect();

    render(NewTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        accounts,
        error: None,
        to_account_number: String::new(),
        amount: String::new(),
        note: String::new(),
    })
}

async fn create(
    form: web::Form<NewTransferForm>,
    svc: web::Data<dyn TransferService>,
    account_svc: web::Data<dyn AccountService>,
    state: web::Data<AppState>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();

    // Helper to re-render the form with an error message and sticky values.
    async fn re_render(
        user: &CurrentUser,
        account_svc: &web::Data<dyn AccountService>,
        form: &NewTransferForm,
        msg: String,
    ) -> Result<HttpResponse, AppError> {
        let selected_id = form.from_account_id;
        let accounts: Vec<AccountOption> = account_svc
            .list_for_user(user.id)
            .await?
            .into_iter()
            .filter(|a| a.status == AccountStatus::Active)
            .map(|a| AccountOption {
                selected: a.id == selected_id,
                account: a,
            })
            .collect();
        render(NewTemplate {
            layout: LayoutCtx::from_user(Some(user)),
            accounts,
            error: Some(msg),
            to_account_number: form.to_account_number.clone(),
            amount: form.amount.clone(),
            note: form.note.clone().unwrap_or_default(),
        })
    }

    // Parse the amount as Decimal.
    let amount = match Decimal::from_str(form.amount.trim()) {
        Ok(d) => d,
        Err(_) => {
            return re_render(&user, &account_svc, &form, "Amount must be a number like 25.00.".into()).await;
        }
    };

    // Authorize: customer can only send from their own account.
    let from = account_svc.get_by_id(form.from_account_id).await?;
    if from.user_id != user.id && user.role == Role::Customer {
        return Err(AppError::Forbidden);
    }

    // Resolve recipient account number → id. We do this directly via the pool
    // rather than extend AccountService — it's a single lookup specific to
    // the transfer flow and isn't worth adding to Member 3's trait.
    let to_account_number = form.to_account_number.trim();
    let to_id: Option<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM accounts WHERE account_number = $1"#,
    )
    .bind(to_account_number)
    .fetch_optional(&state.db)
    .await?;
    let to_id = match to_id {
        Some((id,)) => id,
        None => {
            return re_render(
                &user,
                &account_svc,
                &form,
                format!("Recipient account {to_account_number} not found."),
            )
            .await;
        }
    };

    // Resolve the recipient's name so the confirm page can show who's being paid.
    let to_owner_name: String = sqlx::query_scalar(
        r#"
        SELECT u.full_name
        FROM users u
        JOIN accounts a ON a.user_id = u.id
        WHERE a.id = $1
        "#,
    )
    .bind(to_id)
    .fetch_one(&state.db)
    .await?;

    // Hand off to the service. It enforces amount > 0, from != to, rate limit,
    // and inserts the pending row with an OTP.
    let created = match svc
        .create(user.id, form.from_account_id, to_id, amount, form.note.clone())
        .await
    {
        Ok(c) => c,
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            return re_render(&user, &account_svc, &form, msg).await;
        }
        Err(other) => return Err(other),
    };

    // Render the confirm page directly so we can show the OTP once. Breaks PRG
    // by design — refreshing the confirm page just re-renders it, since money
    // hasn't moved yet.
    render(ConfirmTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        transfer: created.transfer,
        to_account_number: to_account_number.to_string(),
        to_owner_name,
        demo_otp: created.otp,
    })
}

async fn confirm(
    form: web::Form<ConfirmForm>,
    svc: web::Data<dyn TransferService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();
    svc.confirm(user.id, form.transfer_id, form.otp.trim()).await?;

    Ok(HttpResponse::Found()
        .insert_header(("Location", "/transfers"))
        .finish())
}

// ── Render helper ────────────────────────────────────────────────────

fn render<T: Template>(tmpl: T) -> Result<HttpResponse, AppError> {
    let body = tmpl
        .render()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("transfers template: {e}")))?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body))
}
