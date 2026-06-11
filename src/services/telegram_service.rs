//! Telegram OTP channel - Member 4's extended feature.
//!
//! Same OOP shape as the rest of the codebase: the [`OtpChannel`] trait is the
//! abstraction the transfer engine depends on, with two implementations -
//! [`TelegramOtp`] (real out-of-band delivery via the Telegram Bot API) and
//! [`ScreenOtp`] (the on-screen demo fallback). `main.rs` picks one at startup
//! based on whether `TELEGRAM_BOT_TOKEN` is configured, and the engine never
//! knows the difference: classic runtime polymorphism via `Arc<dyn OtpChannel>`.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;

// ── The abstraction ──────────────────────────────────────────────────

#[async_trait]
pub trait OtpChannel: Send + Sync {
    /// Try to deliver the OTP out-of-band. Returns `true` when it reached the
    /// user's linked Telegram; `false` means the caller should fall back to
    /// showing the code on screen (not linked, or delivery failed).
    async fn send_otp(&self, user_id: i64, otp: &str) -> bool;

    /// Deliver a plain notification ("loan approved", "transfer held", …).
    /// Default: not delivered (on-screen toasts still cover it).
    async fn send_note(&self, _user_id: i64, _text: &str) -> bool {
        false
    }
}

/// Fallback impl: never delivers, so the confirm page shows the code on screen
/// (the original demo behaviour). Used when no bot token is configured.
pub struct ScreenOtp;

#[async_trait]
impl OtpChannel for ScreenOtp {
    async fn send_otp(&self, _user_id: i64, _otp: &str) -> bool {
        false
    }
}

// ── The real channel ─────────────────────────────────────────────────

pub struct TelegramOtp {
    // Private - consumers only see the trait. The encapsulation boundary.
    db: PgPool,
    http: reqwest::Client,
    api_base: String,
}

impl TelegramOtp {
    pub fn new(db: PgPool, token: &str) -> Self {
        Self {
            db,
            http: reqwest::Client::new(),
            api_base: format!("https://api.telegram.org/bot{token}"),
        }
    }
}

#[async_trait]
impl OtpChannel for TelegramOtp {
    async fn send_otp(&self, user_id: i64, otp: &str) -> bool {
        // Does this user have a linked chat?
        let chat_id: Option<i64> = match sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT telegram_chat_id FROM users WHERE id = $1"#,
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        {
            Ok(row) => row.flatten(),
            Err(e) => {
                tracing::warn!(user_id, error = %e, "telegram: chat lookup failed");
                None
            }
        };
        let Some(chat_id) = chat_id else { return false };

        let text = format!(
            "\u{1F510} FerroBank verification code: {otp}\n\nEnter it on the \
             confirmation page to continue. If you didn't request a code, \
             sign in and review your account."
        );
        let sent = self
            .http
            .post(format!("{}/sendMessage", self.api_base))
            .json(&json!({ "chat_id": chat_id, "text": text }))
            .timeout(Duration::from_secs(10))
            .send()
            .await;

        match sent {
            Ok(r) if r.status().is_success() => {
                tracing::info!(user_id, "OTP delivered via Telegram");
                true
            }
            Ok(r) => {
                tracing::warn!(user_id, status = %r.status(), "telegram sendMessage refused");
                false
            }
            Err(e) => {
                tracing::warn!(user_id, error = %e, "telegram sendMessage failed");
                false
            }
        }
    }

    async fn send_note(&self, user_id: i64, text: &str) -> bool {
        let chat_id: Option<i64> =
            sqlx::query_scalar::<_, Option<i64>>(r#"SELECT telegram_chat_id FROM users WHERE id = $1"#)
                .bind(user_id)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten()
                .flatten();
        let Some(chat_id) = chat_id else { return false };
        self.http
            .post(format!("{}/sendMessage", self.api_base))
            .json(&json!({ "chat_id": chat_id, "text": text }))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ── Helpers used by main.rs ──────────────────────────────────────────

/// `getMe` → the bot's username (e.g. `FerroBankBot`), used to build the
/// `https://t.me/<username>?start=<code>` deep link on the settings page.
pub async fn bot_username(token: &str) -> Option<String> {
    let resp = reqwest::Client::new()
        .get(format!("https://api.telegram.org/bot{token}/getMe"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    let username = body["result"]["username"].as_str()?.to_string();
    Some(username)
}

/// Long-polls `getUpdates` forever, completing account links.
///
/// When a user taps the deep link and presses Start, Telegram delivers
/// `/start <code>` plus their chat id. We consume the single-use code, store
/// the chat id on the matching user, and reply with a confirmation. Spawned
/// once from `main` when a bot token is configured; any network hiccup just
/// waits and retries.
pub async fn run_link_poller(db: PgPool, token: String) {
    let http = reqwest::Client::new();
    let base = format!("https://api.telegram.org/bot{token}");
    let mut offset: i64 = 0;

    // Register the command menu so users can discover /unlink and /help.
    let _ = http
        .post(format!("{base}/setMyCommands"))
        .json(&json!({ "commands": [
            { "command": "unlink", "description": "Unlink FerroBank - codes appear on screen again" },
            { "command": "help",   "description": "List available commands" }
        ]}))
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    tracing::info!("telegram link poller started");
    loop {
        let resp = http
            .get(format!("{base}/getUpdates"))
            .query(&[("timeout", "30".to_string()), ("offset", offset.to_string())])
            .timeout(Duration::from_secs(40))
            .send()
            .await;

        let body: Option<serde_json::Value> = match resp {
            Ok(r) => r.json().await.ok(),
            Err(e) => {
                tracing::warn!(error = %e, "telegram getUpdates failed; retrying");
                None
            }
        };
        let Some(body) = body else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };

        for update in body["result"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = update["update_id"].as_i64() {
                offset = offset.max(id + 1);
            }
            let chat_id = update["message"]["chat"]["id"].as_i64();
            let text = update["message"]["text"].as_str().unwrap_or("").trim();
            let Some(chat_id) = chat_id else { continue };
            if text.is_empty() {
                continue;
            }

            let reply = if let Some(code) = text.strip_prefix("/start") {
                let code = code.trim();
                if code.is_empty() {
                    "Open FerroBank \u{2192} Settings \u{2192} Telegram codes, and tap \
                     the link there to connect your account.\n\nCommands: /unlink, /help"
                        .to_string()
                } else {
                    // Single use: consume the code, store the chat id.
                    let linked: Option<(String,)> = sqlx::query_as(
                        r#"
                        UPDATE users
                        SET telegram_chat_id = $1, telegram_link_code = NULL
                        WHERE telegram_link_code = $2
                        RETURNING full_name
                        "#,
                    )
                    .bind(chat_id)
                    .bind(code)
                    .fetch_optional(&db)
                    .await
                    .ok()
                    .flatten();

                    match linked {
                        Some((name,)) => {
                            tracing::info!(chat_id, "telegram account linked");
                            format!(
                                "\u{2705} Linked! Hi {name} - your FerroBank one-time \
                                 codes will arrive in this chat from now on.\n\n\
                                 Send /unlink at any time to disconnect."
                            )
                        }
                        None => "That link code wasn't recognised (codes are single-use). \
                                 Generate a fresh link from the FerroBank settings page \
                                 and try again."
                            .to_string(),
                    }
                }
            } else if text.starts_with("/unlink") {
                // Hardened: never instant. The unlink is scheduled 24 hours
                // out so an attacker inside a stolen Telegram account cannot
                // immediately silence future security alerts - the real owner
                // has a day to cancel from the website (password + session
                // required there).
                let scheduled: Vec<(i64,)> = sqlx::query_as(
                    r#"
                    UPDATE users SET telegram_unlink_at = now() + interval '24 hours'
                    WHERE telegram_chat_id = $1 AND telegram_unlink_at IS NULL
                    RETURNING id
                    "#,
                )
                .bind(chat_id)
                .fetch_all(&db)
                .await
                .unwrap_or_default();
                if !scheduled.is_empty() {
                    for (uid,) in &scheduled {
                        crate::services::audit_service::notify(
                            &db,
                            *uid,
                            "Telegram unlink was requested from your chat and is scheduled in 24 hours. If this wasn't you, cancel it in Settings and change your password now.",
                        )
                        .await;
                        let _ = sqlx::query(
                            r#"INSERT INTO audit_log (actor_user_id, event, payload) VALUES ($1, 'telegram.unlink_scheduled', $2)"#,
                        )
                        .bind(uid)
                        .bind(serde_json::json!({ "chat_id": chat_id }))
                        .execute(&db)
                        .await;
                    }
                    tracing::info!(chat_id, count = scheduled.len(), "telegram unlink scheduled");
                    "\u{23F3} Unlink scheduled: this chat stops receiving FerroBank \
                     codes and alerts in 24 hours. If this wasn't you, cancel it \
                     from the FerroBank website (Settings) and change your \
                     password immediately."
                        .to_string()
                } else {
                    "No FerroBank account is linked to this chat, or an unlink is already scheduled.".to_string()
                }
            } else {
                // /help and anything unrecognised.
                "FerroBank bot commands:\n\
                 /start \u{2014} link your FerroBank account (use the button on the Settings page)\n\
                 /unlink \u{2014} stop receiving codes here and unlink this chat\n\
                 /help \u{2014} show this message"
                    .to_string()
            };

            let _ = http
                .post(format!("{base}/sendMessage"))
                .json(&json!({ "chat_id": chat_id, "text": reply }))
                .timeout(Duration::from_secs(10))
                .send()
                .await;
        }
    }
}
