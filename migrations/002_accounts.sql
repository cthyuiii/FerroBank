-- ─────────────────────────────────────────────────────────────────────────────
-- 002_accounts.sql  ·  Accounts module.
--
-- Includes per-account transfer limits and the delayed limit-change ledger:
-- raising a limit only takes effect after a hold window (12 h by default;
-- the seed sets Alice's accounts to 10 s for the demo). The delay means a
-- hijacked session cannot instantly raise limits and drain the account.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE account_type   AS ENUM ('savings', 'checking');
CREATE TYPE account_status AS ENUM ('pending', 'active', 'frozen', 'closed');

CREATE TABLE accounts (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    account_number  TEXT            NOT NULL UNIQUE,
    kind            account_type    NOT NULL,
    status          account_status  NOT NULL DEFAULT 'active',
    balance         NUMERIC(18, 2)  NOT NULL DEFAULT 0,
    -- Maximum amount per single transfer out of this account.
    transfer_limit  NUMERIC(18, 2)  NOT NULL DEFAULT 5000,
    -- Hold window before a requested limit increase takes effect.
    limit_hold_seconds INTEGER      NOT NULL DEFAULT 43200,  -- 12 hours
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT now(),

    CONSTRAINT non_negative_balance CHECK (balance >= 0),
    CONSTRAINT positive_limit       CHECK (transfer_limit > 0)
);

CREATE INDEX accounts_user_id_idx ON accounts (user_id);
CREATE INDEX accounts_status_idx  ON accounts (status);

-- Accounts may only belong to customers — staff never hold bank accounts.
CREATE FUNCTION accounts_owner_must_be_customer() RETURNS trigger AS $$
BEGIN
    IF (SELECT role FROM users WHERE id = NEW.user_id) <> 'customer' THEN
        RAISE EXCEPTION 'accounts may only be opened for customers (user % is not a customer)', NEW.user_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER accounts_owner_customer_check
    BEFORE INSERT OR UPDATE OF user_id ON accounts
    FOR EACH ROW
    EXECUTE FUNCTION accounts_owner_must_be_customer();

-- Pending limit changes. Applied lazily: the transfer engine promotes any
-- matured row before enforcing the limit, so no background job is needed.
CREATE TABLE limit_changes (
    id            BIGSERIAL PRIMARY KEY,
    account_id    BIGINT         NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    old_limit     NUMERIC(18, 2) NOT NULL,
    new_limit     NUMERIC(18, 2) NOT NULL,
    requested_at  TIMESTAMPTZ    NOT NULL DEFAULT now(),
    effective_at  TIMESTAMPTZ    NOT NULL,
    applied_at    TIMESTAMPTZ,

    CONSTRAINT positive_new_limit CHECK (new_limit > 0)
);

CREATE INDEX limit_changes_account_idx ON limit_changes (account_id, effective_at);
