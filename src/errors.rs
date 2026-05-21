//! The single error type every service and handler returns.
//!
//! Implements [`actix_web::ResponseError`] so handlers can use `?` and get
//! correctly-mapped HTTP responses (and a rendered error page) for free.

use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use askama::Template;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("bad request: {0}")]
    BadRequest(String),

    /// Business-rule violation: insufficient funds, frozen account, etc.
    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("record not found".into()),
            other => AppError::Internal(other.into()),
        }
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(err: validator::ValidationErrors) -> Self {
        AppError::BadRequest(format!("validation failed: {err}"))
    }
}

impl From<argon2::password_hash::Error> for AppError {
    fn from(err: argon2::password_hash::Error) -> Self {
        // Don't leak password-hash internals to the user.
        tracing::warn!(error = %err, "argon2 error");
        AppError::Internal(anyhow::anyhow!("password processing failed"))
    }
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage<'a> {
    layout: crate::view::LayoutCtx,
    status: u16,
    title: &'a str,
    message: &'a str,
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        // Unauthorized → bounce to the login page rather than show a 401 dead-end.
        if matches!(self, AppError::Unauthorized) {
            return HttpResponse::Found()
                .insert_header(("Location", "/login"))
                .finish();
        }

        if let AppError::Internal(err) = self {
            tracing::error!(error = ?err, "internal error");
        }

        let status = self.status_code();
        let (title, message) = match self {
            AppError::NotFound(m) => ("Not Found", m.as_str()),
            AppError::Forbidden => (
                "Forbidden",
                "You don't have permission to access this resource.",
            ),
            AppError::BadRequest(m) => ("Bad Request", m.as_str()),
            AppError::Conflict(m) => ("Conflict", m.as_str()),
            AppError::Internal(_) => (
                "Internal Server Error",
                "Something went wrong on our end. Please try again.",
            ),
            AppError::Unauthorized => unreachable!("handled above"),
        };

        let body = ErrorPage {
            layout: crate::view::LayoutCtx::anonymous(),
            status: status.as_u16(),
            title,
            message,
        }
        .render()
        .unwrap_or_else(|_| format!("Error {}: {}", status.as_u16(), message));

        HttpResponse::build(status)
            .content_type("text/html; charset=utf-8")
            .body(body)
    }
}
