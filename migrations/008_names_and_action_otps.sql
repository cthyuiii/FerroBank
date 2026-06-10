-- ─────────────────────────────────────────────────────────────────────────────
-- 008_names_and_action_otps.sql
--
-- Three changes:
--   1. Structured names (first / middle / last) on users. `full_name` stays as
--      the denormalized display string; existing rows are backfilled by
--      splitting it.
--   2. `action_otps` — the generalized OTP guard. The transfer flow already
--      has its own OTP on the transfers row; this table extends one-time-code
--      confirmation to ANY sensitive action: account opening, loan
--      applications, profile changes (e.g. Telegram unlink). One row per
--      pending action; single-use; expires after 10 minutes (enforced in the
--      service).
--   3. Defense-in-depth trigger: transfers must be between two DIFFERENT
--      customers — users can never transfer to themselves (any account they
--      own). The service rejects this first with a friendly message; the
--      trigger guarantees it can't be bypassed.
-- ─────────────────────────────────────────────────────────────────────────────

-- 1. Structured names
ALTER TABLE users
    ADD COLUMN first_name  TEXT,
    ADD COLUMN middle_name TEXT,
    ADD COLUMN last_name   TEXT;

UPDATE users
SET first_name = split_part(full_name, ' ', 1),
    last_name  = NULLIF(btrim(substr(full_name, length(split_part(full_name, ' ', 1)) + 2)), '');

-- 2. Generalized action OTPs
CREATE TABLE action_otps (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Dotted action name, e.g. 'account.open', 'loan.apply', 'telegram.unlink'.
    purpose     TEXT        NOT NULL,
    -- The deferred action's inputs, applied only after the OTP verifies.
    payload     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    -- argon2 hash of the 6-digit code; the plaintext is never stored.
    otp_hash    TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    consumed_at TIMESTAMPTZ
);

CREATE INDEX action_otps_user_idx ON action_otps (user_id, created_at DESC);

-- 3. No self-transfers (cross-user only)
CREATE FUNCTION transfers_must_cross_users() RETURNS trigger AS $$
BEGIN
    IF (SELECT user_id FROM accounts WHERE id = NEW.from_account_id)
     = (SELECT user_id FROM accounts WHERE id = NEW.to_account_id) THEN
        RAISE EXCEPTION 'transfers must be between two different customers';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER transfers_cross_user_check
    BEFORE INSERT ON transfers
    FOR EACH ROW
    EXECUTE FUNCTION transfers_must_cross_users();
