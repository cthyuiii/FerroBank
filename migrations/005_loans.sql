-- ─────────────────────────────────────────────────────────────────────────────
-- 005_loans.sql  ·  Loans module.
--
-- Lifecycle: pending → (teller + admin approvals) → approved → active → paid_off
--            pending → rejected (single staff rejection, or 7-day expiry)
-- On full approval the principal is credited to the borrower's chosen
-- disbursement account inside the same transaction.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE loan_status AS ENUM (
    'pending', 'approved', 'active', 'paid_off', 'rejected'
);

CREATE TABLE loans (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    principal       NUMERIC(18, 2)  NOT NULL,
    interest_rate   NUMERIC(6, 4)   NOT NULL,  -- annual, 0.0525 = 5.25%
    term_months     INTEGER         NOT NULL,
    status          loan_status     NOT NULL DEFAULT 'pending',
    -- Where the principal lands when the loan is fully approved.
    disbursement_account_id BIGINT  REFERENCES accounts(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT now(),
    decided_at      TIMESTAMPTZ,

    CONSTRAINT positive_principal   CHECK (principal > 0),
    CONSTRAINT sane_interest_rate   CHECK (interest_rate >= 0 AND interest_rate < 1),
    CONSTRAINT sane_term            CHECK (term_months >= 1 AND term_months <= 360)
);

CREATE INDEX loans_user_id_idx      ON loans (user_id, created_at DESC);
CREATE INDEX loans_status_idx       ON loans (status);

-- Dual-approval ledger: a loan needs BOTH a 'teller' and an 'admin' row.
CREATE TABLE loan_approvals (
    id                BIGSERIAL    PRIMARY KEY,
    loan_id           BIGINT       NOT NULL REFERENCES loans(id) ON DELETE CASCADE,
    approver_user_id  BIGINT       NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    role              user_role    NOT NULL,
    approved_at       TIMESTAMPTZ  NOT NULL DEFAULT now(),

    CONSTRAINT one_approval_per_role UNIQUE (loan_id, role)
);

CREATE INDEX loan_approvals_loan_idx ON loan_approvals (loan_id);
