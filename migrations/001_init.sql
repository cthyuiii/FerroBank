-- ─────────────────────────────────────────────────────────────────────────────
-- 001_init.sql  ·  Initial schema, owned by the Platform Lead (Member 1).
--
-- Sets up the foundational `users` table and `user_role` enum so the auth
-- middleware compiles and the app boots. Module owners add migrations
-- 002_accounts.sql onwards. Coordinate migration numbers in the team chat
-- before writing.
--
-- Seeding: dev users are NOT inserted here. After running migrations, seed
-- the three dev users with:
--   cargo run --bin seed
-- This goes through the real AuthService::register so the password hashes
-- are produced by the same argon2 code path that production uses.
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
