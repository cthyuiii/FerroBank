-- ─────────────────────────────────────────────────────────────────────────────
-- 001_init.sql  ·  Users & roles (Platform module).
--
-- Consolidated schema: structured names, NRIC identity, Telegram OTP linking,
-- and the activity timestamp used by the 5-minute inactivity guard all live
-- here. Requires a fresh database (drop + recreate, then run the seed).
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE user_role AS ENUM ('customer', 'teller', 'admin');

CREATE TABLE users (
    id              BIGSERIAL PRIMARY KEY,
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    -- Denormalized display string assembled from the parts below.
    full_name       TEXT NOT NULL,
    first_name      TEXT,
    middle_name     TEXT,
    last_name       TEXT,
    -- National ID for customers (staff accounts have none). Staff compare it
    -- against the identity a customer claims during a fraud review.
    nric            TEXT,
    role            user_role NOT NULL DEFAULT 'customer',
    -- Telegram OTP delivery: linked chat + the single-use linking code.
    telegram_chat_id    BIGINT,
    telegram_link_code  TEXT UNIQUE,
    phone_number        TEXT,
    -- Server-side inactivity tracking (5-minute TTL enforced in middleware,
    -- always measured against database time).
    last_activity_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX users_email_idx ON users (email);
