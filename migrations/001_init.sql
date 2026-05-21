-- ─────────────────────────────────────────────────────────────────────────────
-- 001_init.sql  ·  Initial schema, owned by the Platform Lead (Member 1).
--
-- Sets up the foundational `users` table and `user_role` enum so the auth
-- middleware compiles and the app boots. Module owners add migrations
-- 002_users.sql onwards (extending users), 003_accounts.sql, etc.
-- Coordinate migration numbers in the team chat before writing.
-- ─────────────────────────────────────────────────────────────────────────────

-- Role enum. Must match the variants in `src/models/user.rs::Role`.
CREATE TYPE user_role AS ENUM ('customer', 'teller', 'admin');

CREATE TABLE users (
    id              BIGSERIAL PRIMARY KEY,
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    full_name       TEXT NOT NULL,
    role            user_role NOT NULL DEFAULT 'customer',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX users_email_idx ON users (email);

-- ─────────────────────────────────────────────────────────────────────────────
-- Dev seed: three users so login works on day 1 without going through
-- registration. Passwords below are argon2id hashes generated with:
--   echo -n 'admin123'  | argon2 $(openssl rand -base64 16) -id -t 2 -m 16 -p 1
--
-- DEVELOPMENT ONLY. Remove or change before any real deployment.
--   admin@ferrobank.local   / admin123   (admin)
--   teller@ferrobank.local  / teller123  (teller)
--   alice@ferrobank.local   / alice123   (customer)
--
-- The hashes below are placeholders; the Auth module owner (Member 2) will
-- regenerate real ones during their work and update this file in their PR.
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO users (email, password_hash, full_name, role) VALUES
  ('admin@ferrobank.local',  '$argon2id$v=19$m=16,t=2,p=1$REPLACE$REPLACE',  'Admin User',  'admin'),
  ('teller@ferrobank.local', '$argon2id$v=19$m=16,t=2,p=1$REPLACE$REPLACE',  'Teller User', 'teller'),
  ('alice@ferrobank.local',  '$argon2id$v=19$m=16,t=2,p=1$REPLACE$REPLACE',  'Alice Smith', 'customer');
