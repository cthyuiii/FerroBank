-- ─────────────────────────────────────────────────────────────────────────────
-- 006_repayments.sql  ·  Loan repayments (Loans module).
--
-- Append-only payment ledger. The service debits the funding account in the
-- same transaction it records the repayment.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE repayments (
    id          BIGSERIAL PRIMARY KEY,
    loan_id     BIGINT          NOT NULL REFERENCES loans(id) ON DELETE RESTRICT,
    amount      NUMERIC(18, 2)  NOT NULL,
    account_id  BIGINT          REFERENCES accounts(id) ON DELETE SET NULL,
    paid_at     TIMESTAMPTZ     NOT NULL DEFAULT now(),

    CONSTRAINT positive_repayment CHECK (amount > 0)
);

CREATE INDEX repayments_loan_id_idx ON repayments (loan_id, paid_at DESC);
