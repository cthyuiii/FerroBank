-- ─────────────────────────────────────────────────────────────────────────────
-- 007_feature_upgrades.sql
--
-- Adds three capabilities requested after the initial build:
--   1. Loan repayments now record which account the money was drawn from
--      (and the service actually debits that account).
--   2. Transfers can store a human-readable reason for a rejection / flag.
--   3. Dual loan approval: one teller AND one admin must each sign off before
--      a loan moves from 'pending' to 'approved'.
-- ─────────────────────────────────────────────────────────────────────────────

-- 1. Source account for a repayment. Nullable so historical/seed rows are fine.
ALTER TABLE repayments
    ADD COLUMN IF NOT EXISTS account_id BIGINT REFERENCES accounts(id) ON DELETE SET NULL;

-- 2. Why a transfer was rejected or flagged (e.g. "insufficient funds").
ALTER TABLE transfers
    ADD COLUMN IF NOT EXISTS status_reason TEXT;

-- 3. Dual approval ledger. Each loan can collect at most one 'teller' approval
--    and one 'admin' approval (the UNIQUE constraint enforces the slots). Because
--    a user holds exactly one role, filling both slots guarantees two distinct
--    people signed off.
CREATE TABLE IF NOT EXISTS loan_approvals (
    id                BIGSERIAL    PRIMARY KEY,
    loan_id           BIGINT       NOT NULL REFERENCES loans(id)  ON DELETE CASCADE,
    approver_user_id  BIGINT       NOT NULL REFERENCES users(id)  ON DELETE RESTRICT,
    role              user_role    NOT NULL,
    approved_at       TIMESTAMPTZ  NOT NULL DEFAULT now(),

    CONSTRAINT one_approval_per_role UNIQUE (loan_id, role)
);

CREATE INDEX IF NOT EXISTS loan_approvals_loan_idx ON loan_approvals (loan_id);
