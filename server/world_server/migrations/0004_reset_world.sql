-- /resetworld (docs/knowledge/ux/reset-world.md). The world blob as it was before its last reset,
-- for manual recovery; one row per world, overwritten by the next reset.
CREATE TABLE IF NOT EXISTS world_backup (
    name     TEXT PRIMARY KEY,
    data     BYTEA NOT NULL,
    saved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Claims returned by a reset to owners who were not in that world. Paid out (the item, or its
-- store price in gems when there is no room) when the owner next joins a world, and deleted in the
-- same transaction that saves them.
CREATE TABLE IF NOT EXISTS returned_claim (
    id        BIGSERIAL PRIMARY KEY,
    player_id BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    item_id   INT NOT NULL CHECK (item_id BETWEEN 1 AND 65535),
    price     BIGINT NOT NULL CHECK (price >= 0),
    notice    TEXT NOT NULL
);
