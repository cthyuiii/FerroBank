-- ─────────────────────────────────────────────────────────────────────────────
-- 009_telegram_unlink_delay.sql  ·  Hardened unlink (Auth module).
--
-- The bot's /unlink no longer disconnects instantly: it schedules the unlink
-- 24 hours out and warns the owner everywhere. An attacker inside a stolen
-- Telegram account can no longer silence future alerts immediately - the real
-- owner has a day to cancel from the website (which requires their password
-- and session). The website's own OTP-confirmed unlink stays immediate.
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE users
    ADD COLUMN telegram_unlink_at TIMESTAMPTZ;
