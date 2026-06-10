-- ─────────────────────────────────────────────────────────────────────────────
-- 008_loan_due_dates.sql  ·  Repayment due dates (Loans module).
--
-- Monthly repayment cadence: set to approval + 1 month when the loan is fully
-- approved, then advanced by 1 month after every repayment until paid off.
-- Surfaced on the loan page and in Telegram/browser notifications.
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE loans
    ADD COLUMN next_payment_due TIMESTAMPTZ;
