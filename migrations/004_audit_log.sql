-- ─────────────────────────────────────────────────────────────────────────────
-- 004_audit_log.sql  ·  Audit log table, owned by Member 4 (Transfers module).
--
-- Append-only event log. Any significant security or financial action writes
-- a row here via AuditService::record(actor, event, payload).
--
-- Event names are dotted, e.g. "transfer.completed", "loan.approved",
-- "auth.login.success", "account.frozen". Payload is free-form JSONB.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE audit_log (
    id              BIGSERIAL PRIMARY KEY,
    actor_user_id   BIGINT REFERENCES users(id) ON DELETE SET NULL,
    event           TEXT        NOT NULL,
    payload         JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_log_event_idx        ON audit_log (event, created_at DESC);
CREATE INDEX audit_log_actor_idx        ON audit_log (actor_user_id, created_at DESC);
CREATE INDEX audit_log_created_at_idx   ON audit_log (created_at DESC);
