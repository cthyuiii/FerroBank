-- ─────────────────────────────────────────────────────────────────────────────
-- 005_loans.sql  ·  Loan applications, owned by Member 5.
--
-- Loan lifecycle:
--   pending  → submitted by a customer, awaiting staff review
--   approved → admin approved but no repayments yet (lump-sum disbursed)
--   active   → at least one repayment recorded, still outstanding
--   paid_off → outstanding balance reached zero
--   rejected → admin rejected the application
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE loan_status AS ENUM (
    'pending', 'approved', 'active', 'paid_off', 'rejected'
);

CREATE TABLE loans (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    principal       NUMERIC(18, 2)  NOT NULL,
    -- Annual interest rate as a decimal. 0.0525 = 5.25%.
    interest_rate   NUMERIC(6, 4)   NOT NULL,
    term_months     INTEGER         NOT NULL,
    status          loan_status     NOT NULL DEFAULT 'pending',
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT now(),
    decided_at      TIMESTAMPTZ,

    CONSTRAINT positive_principal   CHECK (principal > 0),
    CONSTRAINT sane_interest_rate   CHECK (interest_rate >= 0 AND interest_rate < 1),
    CONSTRAINT sane_term            CHECK (term_months >= 1 AND term_months <= 360)
);

CREATE INDEX loans_user_id_idx      ON loans (user_id, created_at DESC);
CREATE INDEX loans_status_idx       ON loans (status);
