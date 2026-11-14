-- Multi-server ownership (docs/knowledge/10, "Scaling"): each server holds at most 500 players,
-- and every server shares one PostgreSQL. A world and a player each belong to exactly one server at
-- a time, so no world runs twice and no account is online twice (both would let items duplicate).
-- A lease that is not renewed expires, so a crashed server's worlds become free after 30 s.

CREATE TABLE IF NOT EXISTS world_lease (
    world      TEXT PRIMARY KEY,
    server_id  TEXT NOT NULL,
    addr       TEXT NOT NULL,          -- host:port the client is redirected to
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS player_session (
    player_id  BIGINT PRIMARY KEY,     -- account id from the login service
    server_id  TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

-- Live servers and their load, for picking a server under 500 at login.
CREATE TABLE IF NOT EXISTS game_server (
    server_id  TEXT PRIMARY KEY,
    addr       TEXT NOT NULL,
    online     INT  NOT NULL CHECK (online BETWEEN 0 AND 500),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Queries the server runs ($1 = key, $2 = this server_id, $3 = addr):
--
-- Claim a world. One row back = ours (new, expired, or already ours); no row = held elsewhere,
-- then SELECT addr FROM world_lease WHERE world = $1 and redirect the client there.
--   INSERT INTO world_lease VALUES ($1, $2, $3, now() + interval '30 seconds')
--   ON CONFLICT (world) DO UPDATE
--     SET server_id = $2, addr = $3, expires_at = now() + interval '30 seconds'
--     WHERE world_lease.expires_at < now() OR world_lease.server_id = $2
--   RETURNING world;
--
-- Renew every 10 s, for all worlds this server holds:
--   UPDATE world_lease SET expires_at = now() + interval '30 seconds' WHERE server_id = $2;
--
-- Release on unload, after the final save:
--   DELETE FROM world_lease WHERE world = $1 AND server_id = $2;
--
-- player_session uses the same three queries keyed on player_id; claim at Hello, refuse the
-- login ("already online") when no row comes back.
--
-- Heartbeat every 10 s; the login service picks the least loaded live server:
--   INSERT INTO game_server VALUES ($2, $3, $4, now() + interval '30 seconds')
--   ON CONFLICT (server_id) DO UPDATE SET addr = $3, online = $4, expires_at = now() + interval '30 seconds';
--   SELECT addr FROM game_server WHERE expires_at > now() AND online < 500 ORDER BY online LIMIT 1;
