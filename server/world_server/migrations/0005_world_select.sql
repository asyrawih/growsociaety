-- World select (docs/knowledge/ux/world-select.md). One row per world when it is first stored
-- (an untouched new world never is): who opened it and when, for the per-account creation limit.
CREATE TABLE IF NOT EXISTS world_created (
    name       TEXT PRIMARY KEY,
    player_id  BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS world_created_by_player ON world_created (player_id, created_at);
