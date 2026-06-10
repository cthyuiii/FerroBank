-- ─────────────────────────────────────────────────────────────────────────────
-- 004_audit_log.sql  ·  Audit log + user notifications (Platform module).
--
-- audit_log: append-only event trail. Every significant security or financial
-- action writes a row via AuditService::record(actor, event, payload).
-- notifications: per-user messages ("loan approved", "transfer held", …)
-- surfaced as browser toasts and, when linked, as Telegram messages.
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

CREATE TABLE notifications (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    message     TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    seen_at     TIMESTAMPTZ
);

CREATE INDEX notifications_user_idx ON notifications (user_id, seen_at, created_at DESC);
