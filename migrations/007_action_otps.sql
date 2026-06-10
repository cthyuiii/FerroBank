-- ─────────────────────────────────────────────────────────────────────────────
-- 007_action_otps.sql  ·  Generalized OTP guard (Transfers module).
--
-- One-time-code confirmation for every sensitive action beyond transfers:
-- account opening, loan applications, profile changes (email, password,
-- Telegram unlink). One row per pending action; single-use; the service
-- enforces a 10-minute TTL.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE action_otps (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose     TEXT        NOT NULL,   -- 'account.open', 'loan.apply', 'profile.email', …
    payload     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    otp_hash    TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    consumed_at TIMESTAMPTZ
);

CREATE INDEX action_otps_user_idx ON action_otps (user_id, created_at DESC);
