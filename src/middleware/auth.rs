//! Session-backed authentication: a `CurrentUser` extractor for handlers and a
//! `RequireRole` middleware for protecting whole scopes.
//!
//! What the Platform Lead publishes:
//! - [`CurrentUser`] - drop into any handler signature to require a logged-in user.
//! - [`RequireRole`] - `.wrap(RequireRole(Role::Admin))` on a scope to enforce a role.
//! - [`session::login`] / [`session::logout`] - for the Auth module to flip session state.
//!
//! Auth module owner uses `session::login(&session, SessionUser { ... })` after
//! verifying credentials, and `session::logout(&session)` on logout.

use std::future::{ready, Ready};

use actix_session::SessionExt;
use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, FromRequest, HttpRequest, HttpResponse,
};
use futures::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};

use crate::errors::AppError;
use crate::models::user::Role;

/// Session storage key. Kept private so nobody outside this module writes it directly.
const SESSION_KEY: &str = "user";

/// What we store in the session cookie. Kept minimal - anything else should be
/// fetched from the DB via the user id.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionUser {
    pub id: i64,
    pub email: String,
    /// Given name, for greetings ("Hello, Alice").
    pub name: String,
    pub role: Role,
}

/// Extractor: drop into any handler signature to require a logged-in user.
///
/// If the session cookie is missing or invalid, the handler short-circuits with
/// `AppError::Unauthorized`, which the global error mapper turns into a redirect
/// to `/login`.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    /// Given name, for greetings.
    pub name: String,
    pub role: Role,
}

impl FromRequest for CurrentUser {
    type Error = AppError;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        let session = req.get_session();
        let result = match session.get::<SessionUser>(SESSION_KEY) {
            Ok(Some(u)) => Ok(CurrentUser {
                id: u.id,
                email: u.email,
                name: u.name,
                role: u.role,
            }),
            _ => Err(AppError::Unauthorized),
        };
        ready(result)
    }
}

// ── Session helpers (used by the Auth module) ─────────────────────────

pub mod session {
    use super::{SessionUser, SESSION_KEY};
    use crate::errors::AppError;
    use actix_session::Session;

    /// Persist the user into the session cookie. Call this after verifying credentials.
    pub fn login(session: &Session, user: SessionUser) -> Result<(), AppError> {
        session
            .insert(SESSION_KEY, user)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("session insert failed: {e}")))
    }

    /// Wipe the session. Call this from the logout handler.
    pub fn logout(session: &Session) {
        session.purge();
    }
}

// ── Role guard middleware ─────────────────────────────────────────────

/// Wrap a scope with `RequireRole(Role::Admin)` to enforce role-based access.
///
/// `Role::Admin` is treated as a superuser - admins pass any `RequireRole` check.
/// Anonymous visitors are redirected to `/login`; logged-in users without the role
/// get a 403.
pub struct RequireRole(pub Role);

impl<S, B> Transform<S, ServiceRequest> for RequireRole
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = RequireRoleMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireRoleMiddleware {
            service,
            required: self.0.clone(),
        }))
    }
}

pub struct RequireRoleMiddleware<S> {
    service: S,
    required: Role,
}

impl<S, B> Service<ServiceRequest> for RequireRoleMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let session = req.get_session();
        let session_user = session.get::<SessionUser>(SESSION_KEY).ok().flatten();
        let required = self.required.clone();

        match session_user {
            // Admin is a superuser. Otherwise the role must match exactly.
            Some(u) if u.role == Role::Admin || u.role == required => {
                let fut = self.service.call(req);
                Box::pin(async move {
                    let res = fut.await?;
                    Ok(res.map_into_left_body())
                })
            }
            Some(_) => {
                let (req, _) = req.into_parts();
                let resp = crate::errors::forbidden_page();
                Box::pin(async move {
                    Ok(ServiceResponse::new(req, resp).map_into_right_body())
                })
            }
            None => {
                let (req, _) = req.into_parts();
                let resp = HttpResponse::Found()
                    .insert_header(("Location", "/login"))
                    .finish();
                Box::pin(async move {
                    Ok(ServiceResponse::new(req, resp).map_into_right_body())
                })
            }
        }
    }
}

// ── Activity guard ───────────────────────────────────────────────────

/// Global middleware wrapped around the whole app. Three jobs:
///
/// 1. **Activity trail** - logs every authenticated request (method, path,
///    user id), so GET/POST/PUT/DELETE traffic is traceable per user.
/// 2. **Inactivity TTL** - 5 minutes, measured against DATABASE time via
///    `users.last_activity_at`, so it can't be bypassed by tampering with
///    browser cookies. Expired sessions are purged and redirected to
///    `/login?expired=1` on their next request, whatever it was.
/// 3. **Mandatory Telegram link** - when Telegram OTP is configured,
///    customers are routed to `/settings/telegram` and nowhere else until
///    their account is linked (codes and account updates arrive there).
pub struct ActivityGuard;

impl<S, B> Transform<S, ServiceRequest> for ActivityGuard
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = ActivityGuardMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ActivityGuardMiddleware {
            service: std::rc::Rc::new(service),
        }))
    }
}

pub struct ActivityGuardMiddleware<S> {
    service: std::rc::Rc<S>,
}

impl<S, B> Service<ServiceRequest> for ActivityGuardMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = std::rc::Rc::clone(&self.service);
        Box::pin(async move {
            let session = req.get_session();
            let su = session.get::<SessionUser>(SESSION_KEY).ok().flatten();

            if let Some(u) = su {
                let method = req.method().to_string();
                let path = req.path().to_string();
                // (1) Per-user activity trail in the server log.
                tracing::info!(user_id = u.id, %method, %path, "user activity");

                if let Some(state) = req.app_data::<actix_web::web::Data<crate::state::AppState>>() {
                    let db = state.db.clone();
                    let telegram_enabled = state.telegram_bot.is_some();

                    // (2) Inactivity TTL on database time.
                    let expired: Option<(bool,)> = sqlx::query_as(
                        r#"
                        SELECT (last_activity_at IS NOT NULL
                                AND last_activity_at < now() - interval '5 minutes')
                        FROM users WHERE id = $1
                        "#,
                    )
                    .bind(u.id)
                    .fetch_optional(&db)
                    .await
                    .ok()
                    .flatten();
                    if matches!(expired, Some((true,))) {
                        session.purge();
                        let (req, _) = req.into_parts();
                        let resp = HttpResponse::Found()
                            .insert_header(("Location", "/login?expired=1"))
                            .finish();
                        return Ok(ServiceResponse::new(req, resp).map_into_right_body());
                    }
                    let _ = sqlx::query(
                        r#"UPDATE users SET last_activity_at = now() WHERE id = $1"#,
                    )
                    .bind(u.id)
                    .execute(&db)
                    .await;

                    // Apply a matured 24h bot-scheduled unlink (lazy, cheap:
                    // a single-row no-op unless one is actually due).
                    let _ = sqlx::query(
                        r#"
                        UPDATE users SET telegram_chat_id = NULL, telegram_unlink_at = NULL
                        WHERE id = $1 AND telegram_unlink_at IS NOT NULL AND telegram_unlink_at <= now()
                        "#,
                    )
                    .bind(u.id)
                    .execute(&db)
                    .await;

                    // (3) Customers must finish Telegram linking first.
                    if telegram_enabled && u.role == Role::Customer {
                        let allowed = path.starts_with("/settings/telegram")
                            || path == "/logout"
                            || path == "/notifications";
                        if !allowed {
                            let linked: Option<(bool,)> = sqlx::query_as(
                                r#"SELECT telegram_chat_id IS NOT NULL FROM users WHERE id = $1"#,
                            )
                            .bind(u.id)
                            .fetch_optional(&db)
                            .await
                            .ok()
                            .flatten();
                            if !matches!(linked, Some((true,))) {
                                let (req, _) = req.into_parts();
                                let resp = HttpResponse::Found()
                                    .insert_header(("Location", "/settings/telegram"))
                                    .finish();
                                return Ok(ServiceResponse::new(req, resp).map_into_right_body());
                            }
                        }
                    }
                }
            }

            let res = service.call(req).await?;
            Ok(res.map_into_left_body())
        })
    }
}
