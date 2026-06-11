-- ─────────────────────────────────────────────────────────────────────────────
-- 008_login_sessions.sql  ·  Login device tracking (Auth module).
--
-- One row per successful sign-in: browser family, raw user agent, and source
-- IP. First-seen browsers and networks are flagged so fraud review can spot
-- account takeover, and the admin per-user view lists the full history.
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE login_sessions (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    browser         TEXT        NOT NULL,  -- Chrome / Safari / Firefox / Edge / Other
    user_agent      TEXT,
    ip              TEXT,
    is_new_device   BOOLEAN     NOT NULL DEFAULT false,
    is_new_network  BOOLEAN     NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX login_sessions_user_idx ON login_sessions (user_id, created_at DESC);
