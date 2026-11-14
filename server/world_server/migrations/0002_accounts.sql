-- Accounts (docs/knowledge/10, "Authentication"). `id` is the player's id in the game.
-- Names are unique ignoring case; the CHECKs repeat the server's own rules so a bug cannot store
-- a bad row. Roles are set by an operator in SQL (docs/knowledge/13: making an Admin is not a command).
CREATE TABLE IF NOT EXISTS account (
    id            BIGSERIAL PRIMARY KEY,
    name          TEXT NOT NULL CHECK (name ~ '^[A-Za-z0-9_]{1,16}$'),
    password_hash TEXT NOT NULL CHECK (password_hash LIKE '$argon2id$%'),
    role          TEXT NOT NULL DEFAULT 'player' CHECK (role IN ('player', 'moderator', 'admin')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS account_name_lower ON account (lower(name));
