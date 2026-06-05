-- ─────────────────────────────────────────────────────────────────────────────
-- 003_transfers.sql  ·  Transfers module schema, owned by Member 4.
--
-- A transfer is a two-step operation:
--   1. `create()` — insert row with status='pending' and an OTP hash.
--   2. `confirm()` — verify OTP, lock both account rows FOR UPDATE,
--      move money, set status='completed'.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE transfer_status AS ENUM ('pending', 'completed', 'failed', 'rejected');

CREATE TABLE transfers (
    id                  BIGSERIAL PRIMARY KEY,
    from_account_id     BIGINT          NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    to_account_id       BIGINT          NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    amount              NUMERIC(18, 2)  NOT NULL,
    status              transfer_status NOT NULL DEFAULT 'pending',
    note                TEXT,
    -- Human-readable reason a transfer was rejected or flagged (e.g.
    -- "insufficient funds"). NULL for ordinary completed/pending transfers.
    status_reason       TEXT,
    -- argon2 hash of the 6-digit OTP. NULL once the transfer has been confirmed.
    otp_hash            TEXT,
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT now(),
    confirmed_at        TIMESTAMPTZ,

    CONSTRAINT positive_amount      CHECK (amount > 0),
    CONSTRAINT no_self_transfer     CHECK (from_account_id <> to_account_id)
);

-- history() queries join transfers ⇄ accounts on either side and order by created_at.
CREATE INDEX transfers_from_idx     ON transfers (from_account_id, created_at DESC);
CREATE INDEX transfers_to_idx       ON transfers (to_account_id,   created_at DESC);
CREATE INDEX transfers_status_idx   ON transfers (status);
