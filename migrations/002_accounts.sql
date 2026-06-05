-- ─────────────────────────────────────────────────────────────────────────────
-- 002_accounts.sql  ·  Accounts module schema, owned by Member 3.
--
-- Adds the `accounts` table plus its enums. Designed so the Transfers module
-- (Member 4) can reference accounts.id via FK in migration 004_transfers.sql.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE account_type   AS ENUM ('savings', 'checking');
-- 'pending' = opened by a customer, awaiting staff (teller/admin) approval before
-- it becomes 'active' and can transact.
CREATE TYPE account_status AS ENUM ('pending', 'active', 'frozen', 'closed');

CREATE TABLE accounts (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    account_number  TEXT            NOT NULL UNIQUE,
    kind            account_type    NOT NULL,
    status          account_status  NOT NULL DEFAULT 'active',
    balance         NUMERIC(18, 2)  NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT now(),

    CONSTRAINT non_negative_balance CHECK (balance >= 0)
);

CREATE INDEX accounts_user_id_idx ON accounts (user_id);
CREATE INDEX accounts_status_idx  ON accounts (status);

-- ─────────────────────────────────────────────────────────────────────────────
-- Accounts may only belong to *customers* — staff (teller/admin) never hold
-- bank accounts. A plain CHECK can't reference another table, so this is
-- enforced with a trigger that rejects any account whose owner isn't a customer.
-- ─────────────────────────────────────────────────────────────────────────────
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
