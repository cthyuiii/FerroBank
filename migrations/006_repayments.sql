-- ─────────────────────────────────────────────────────────────────────────────
-- 006_repayments.sql  ·  Loan repayments, owned by Member 5.
--
-- Append-only ledger of payments against a loan. Outstanding balance is
-- derived as `principal + total_interest - SUM(repayments.amount)` where
-- total_interest comes from the amortization formula in loan_service.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE repayments (
    id          BIGSERIAL PRIMARY KEY,
    loan_id     BIGINT          NOT NULL REFERENCES loans(id) ON DELETE RESTRICT,
    amount      NUMERIC(18, 2)  NOT NULL,
    paid_at     TIMESTAMPTZ     NOT NULL DEFAULT now(),

    CONSTRAINT positive_repayment CHECK (amount > 0)
);

CREATE INDEX repayments_loan_id_idx ON repayments (loan_id, paid_at DESC);
