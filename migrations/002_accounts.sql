-- ─────────────────────────────────────────────────────────────────────────────
-- 002_accounts.sql  ·  Accounts module schema, owned by Member 3.
--
-- Adds the `accounts` table plus its enums. Designed so the Transfers module
-- (Member 4) can reference accounts.id via FK in migration 004_transfers.sql.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE account_type   AS ENUM ('savings', 'checking');
CREATE TYPE account_status AS ENUM ('active', 'frozen', 'closed');

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
