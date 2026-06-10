-- ─────────────────────────────────────────────────────────────────────────────
-- 007_telegram.sql  ·  Telegram OTP linking, owned by Member 4 (Transfers).
--
-- Linking flow:
--   1. /settings/telegram generates a single-use `telegram_link_code` and shows
--      a deep link  https://t.me/<bot>?start=<code>.
--   2. The user taps Start; Telegram delivers "/start <code>" + their chat id
--      to the bot. The background poller matches the code, stores the chat id,
--      and clears the code (single use).
--   3. From then on, transfer OTPs are sent to `telegram_chat_id` instead of
--      being shown on screen.
--
-- `telegram_chat_id` is deliberately NOT unique: in a class demo several seed
-- users may link to the same grader's Telegram account.
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE users
    ADD COLUMN phone_number       TEXT,
    ADD COLUMN telegram_chat_id   BIGINT,
    ADD COLUMN telegram_link_code TEXT UNIQUE;
