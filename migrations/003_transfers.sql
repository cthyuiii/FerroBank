-- ─────────────────────────────────────────────────────────────────────────────
-- 003_transfers.sql  ·  Transfers module.
--
-- Two-step OTP flow (create → confirm) plus the fraud-hold pipeline:
--   pending  → awaiting OTP confirmation (max 3 attempts, 10-minute expiry)
--   on_hold  → fraud rules tripped at confirm time; money NOT moved; the
--              customer submits a purpose + identity claim (transfer_reviews)
--              and a teller/admin releases or denies it
--   completed / rejected / failed → terminal states
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE transfer_status AS ENUM ('pending', 'completed', 'failed', 'rejected', 'on_hold');

CREATE TABLE transfers (
    id                  BIGSERIAL PRIMARY KEY,
    from_account_id     BIGINT          NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    to_account_id       BIGINT          NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    amount              NUMERIC(18, 2)  NOT NULL,
    status              transfer_status NOT NULL DEFAULT 'pending',
    note                TEXT,
    status_reason       TEXT,
    -- argon2 hash of the 6-digit OTP; NULL once confirmed.
    otp_hash            TEXT,
    -- Failed OTP entries so far; 3 strikes rejects the transfer.
    otp_attempts        INTEGER         NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT now(),
    confirmed_at        TIMESTAMPTZ,

    CONSTRAINT positive_amount      CHECK (amount > 0),
    CONSTRAINT no_self_transfer     CHECK (from_account_id <> to_account_id)
);

CREATE INDEX transfers_from_idx     ON transfers (from_account_id, created_at DESC);
CREATE INDEX transfers_to_idx       ON transfers (to_account_id,   created_at DESC);
CREATE INDEX transfers_status_idx   ON transfers (status);

-- No self-transfers at all: both accounts must belong to different users.
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

-- One review request per held transfer: the customer's stated purpose and
-- claimed identity, plus the staff decision once made.
CREATE TABLE transfer_reviews (
    id            BIGSERIAL PRIMARY KEY,
    transfer_id   BIGINT      NOT NULL UNIQUE REFERENCES transfers(id) ON DELETE CASCADE,
    purpose       TEXT        NOT NULL,
    nric_claimed  TEXT        NOT NULL,
    submitted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by    BIGINT      REFERENCES users(id),
    decided_at    TIMESTAMPTZ,
    decision      TEXT  -- 'released' | 'denied'
);
