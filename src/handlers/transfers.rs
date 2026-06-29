//! Transfers handlers
//!
//! Two-step flow:
//!   GET  /transfers           → user's transfer history
//!   GET  /transfers/new       → form (accepts ?from=<id> to pre-select)
//!   POST /transfers/new       → create pending row + render confirm page (with OTP)
//!   POST /transfers/confirm   → verify OTP, move money inside one SQL txn, redirect
//!
//! One-time codes are delivered to the sender's linked Telegram; the on-screen
//! code only appears when no Telegram bot is configured (demo fallback).

use actix_web::{web, HttpResponse};
use askama::Template;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

use crate::errors::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::account::{Account, AccountStatus};
use crate::models::transfer::{Transfer, TransferStatus};
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
            .route("/confirm", web::post().to(confirm))
            .route("/{id}/review", web::get().to(review_form))
            .route("/{id}/review", web::post().to(review_submit))
            .route("/{id}/status", web::get().to(status)),
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
    /// Inline error (e.g. an invalid code) shown on the same page.
    error: Option<String>,
    transfer: Transfer,
    /// Recipient account number (as typed) and resolved owner name, so the
    /// sender can verify who they're paying before confirming.
    to_account_number: String,
    to_owner_name: String,
    /// `Some(code)` → show on screen (demo fallback);
    /// `None` → the code went to the user's linked Telegram.
    demo_otp: Option<String>,
}

#[derive(Template)]
#[template(path = "transfers/history.html")]
struct HistoryTemplate {
    layout: LayoutCtx,
    rows: Vec<HistoryRow>,
}

/// One history row, pre-resolved to human-readable details so the customer view
/// never shows raw account/user ids - just the counterparty's name and the
/// account that sent or received the money.
struct HistoryRow {
    id: i64,
    amount: rust_decimal::Decimal,
    status: TransferStatus,
    /// Rejection reason to show (specific reason, or a contact-admin default).
    reason: Option<String>,
    note: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    /// True when the money left one of the viewer's own accounts.
    outgoing: bool,
    /// The viewer's own account number involved in this transfer.
    own_account: String,
    counterparty_name: String,
    counterparty_account: String,
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
    state: web::Data<AppState>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let transfers = svc.history(user.id).await?;

    let owned: std::collections::HashSet<i64> = account_svc
        .list_for_user(user.id)
        .await?
        .into_iter()
        .map(|a| a.id)
        .collect();

    // Resolve every account referenced (either side) to its number + owner name.
    let mut ids: Vec<i64> = Vec::new();
    for t in &transfers {
        ids.push(t.from_account_id);
        ids.push(t.to_account_id);
    }
    ids.sort_unstable();
    ids.dedup();

    let mut info: std::collections::HashMap<i64, (String, String)> =
        std::collections::HashMap::new();
    if !ids.is_empty() {
        let lookup: Vec<(i64, String, String)> = sqlx::query_as(
            r#"
            SELECT a.id, a.account_number, u.full_name
            FROM accounts a
            JOIN users u ON u.id = a.user_id
            WHERE a.id = ANY($1)
            "#,
        )
        .bind(ids.as_slice())
        .fetch_all(&state.db)
        .await?;
        for (id, number, name) in lookup {
            info.insert(id, (number, name));
        }
    }

    let rows: Vec<HistoryRow> = transfers
        .into_iter()
        .map(|t| {
            let outgoing = owned.contains(&t.from_account_id);
            let (own_id, cp_id) = if outgoing {
                (t.from_account_id, t.to_account_id)
            } else {
                (t.to_account_id, t.from_account_id)
            };
            let own_account = info.get(&own_id).map(|x| x.0.clone()).unwrap_or_default();
            let (counterparty_account, counterparty_name) = info
                .get(&cp_id)
                .cloned()
                .unwrap_or_else(|| (String::new(), "Unknown".to_string()));
            // Specific reason if we have one; otherwise a short contact-admin
            // default for anything rejected/failed.
            let reason = t.status_reason.clone().or_else(|| {
                if matches!(t.status, TransferStatus::Rejected | TransferStatus::Failed) {
                    Some("Please contact an administrator.".to_string())
                } else {
                    None
                }
            });
            HistoryRow {
                id: t.id,
                amount: t.amount,
                status: t.status,
                reason,
                note: t.note,
                created_at: t.created_at,
                outgoing,
                own_account,
                counterparty_name,
                counterparty_account,
            }
        })
        .collect();

    render(HistoryTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        rows,
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
    // rather than extend AccountService - it's a single lookup specific to
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
    // by design - refreshing the confirm page just re-renders it, since money
    // hasn't moved yet.
    render(ConfirmTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        error: None,
        transfer: created.transfer,
        to_account_number: to_account_number.to_string(),
        to_owner_name,
        demo_otp: if created.otp_delivered {
            None
        } else {
            Some(created.otp)
        },
    })
}

async fn confirm(
    form: web::Form<ConfirmForm>,
    svc: web::Data<dyn TransferService>,
    state: web::Data<AppState>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let form = form.into_inner();
    let transfer = match svc.confirm(user.id, form.transfer_id, form.otp.trim()).await {
        Ok(t) => t,
        // Wrong code with attempts left: stay on the confirm page and say so.
        Err(AppError::BadRequest(msg)) if msg.starts_with("invalid confirmation code") => {
            let transfer = svc.get_for_owner(user.id, form.transfer_id).await?;
            let (to_account_number, to_owner_name): (String, String) = sqlx::query_as(
                r#"
                SELECT a.account_number, u.full_name
                FROM accounts a JOIN users u ON u.id = a.user_id
                WHERE a.id = $1
                "#,
            )
            .bind(transfer.to_account_id)
            .fetch_one(&state.db)
            .await?;
            return render(ConfirmTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                error: Some(msg),
                transfer,
                to_account_number,
                to_owner_name,
                demo_otp: None,
            });
        }
        // Lockout / expiry / already-processed: the transfer is settled.
        Err(AppError::BadRequest(_)) | Err(AppError::Conflict(_)) => {
            return Ok(HttpResponse::Found()
                .insert_header(("Location", "/transfers"))
                .finish());
        }
        Err(other) => return Err(other),
    };

    // Fraud rules may have parked it: send the user to the review form so
    // they can state the purpose and prove their identity.
    if transfer.status == TransferStatus::OnHold {
        return Ok(HttpResponse::Found()
            .insert_header(("Location", format!("/transfers/{}/review", transfer.id)))
            .finish());
    }

    Ok(HttpResponse::Found()
        .insert_header(("Location", "/transfers"))
        .finish())
}

// ── Held-transfer review (customer side) ─────────────────────────────

#[derive(Template)]
#[template(path = "transfers/review.html")]
struct ReviewTemplate {
    layout: LayoutCtx,
    transfer: Transfer,
    error: Option<String>,
    submitted: bool,
}

#[derive(Debug, Deserialize)]
struct ReviewForm {
    purpose: String,
    nric: String,
}

async fn review_form(
    path: web::Path<i64>,
    svc: web::Data<dyn TransferService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let transfer = svc.get_for_owner(user.id, path.into_inner()).await?;
    render(ReviewTemplate {
        layout: LayoutCtx::from_user(Some(&user)),
        transfer,
        error: None,
        submitted: false,
    })
}

async fn review_submit(
    path: web::Path<i64>,
    form: web::Form<ReviewForm>,
    svc: web::Data<dyn TransferService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let transfer_id = path.into_inner();
    match svc
        .submit_review(user.id, transfer_id, &form.purpose, &form.nric)
        .await
    {
        Ok(()) => {
            let transfer = svc.get_for_owner(user.id, transfer_id).await?;
            render(ReviewTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                transfer,
                error: None,
                submitted: true,
            })
        }
        Err(AppError::BadRequest(msg)) | Err(AppError::Conflict(msg)) => {
            let transfer = svc.get_for_owner(user.id, transfer_id).await?;
            render(ReviewTemplate {
                layout: LayoutCtx::from_user(Some(&user)),
                transfer,
                error: Some(msg),
                submitted: false,
            })
        }
        Err(other) => Err(other),
    }
}

/// Live status for the review page's auto-redirect: the moment staff release
/// or deny the held transfer, the page leaves on its own.
async fn status(
    path: web::Path<i64>,
    svc: web::Data<dyn TransferService>,
    user: CurrentUser,
) -> Result<HttpResponse, AppError> {
    let t = svc.get_for_owner(user.id, path.into_inner()).await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "status": t.status.label() })))
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
