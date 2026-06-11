-- ─────────────────────────────────────────────────────────────────────────────
-- 010_otp_attempts_and_origin_blocks.sql  ·  OTP strikes everywhere (Auth).
--
-- 1. action_otps gains a per-attempt counter: every OTP-gated action now has
--    the same 3-strike rule the transfer flow always had. The third wrong
--    code consumes the action entirely.
-- 2. blocked_origins: when a risk-based step-up login fails 3 times, that
--    browser + network combination is blocked from signing in to the account
--    for 24 hours - the brute-force window on a 6-digit code drops from
--    unlimited tries to three.
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE action_otps
    ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

CREATE TABLE blocked_origins (
    id            BIGSERIAL PRIMARY KEY,
    user_id       BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    browser       TEXT        NOT NULL,
    ip            TEXT        NOT NULL,
    reason        TEXT        NOT NULL,
    blocked_until TIMESTAMPTZ NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX blocked_origins_lookup_idx
    ON blocked_origins (user_id, browser, ip, blocked_until);
