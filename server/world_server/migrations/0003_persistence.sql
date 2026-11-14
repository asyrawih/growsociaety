-- Persistence (docs/knowledge/10, "Persistence"). A world is one versioned blob (`store::encode_world`);
-- a player's inventory is rows, so the database itself refuses 201 of an item or two rows for one
-- item. A world and the inventories of the players in it are saved in one transaction.

CREATE TABLE IF NOT EXISTS world (
    name     TEXT PRIMARY KEY CHECK (name ~ '^[A-Z0-9]{1,24}$'),
    data     BYTEA NOT NULL,
    saved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS player_state (
    player_id BIGINT PRIMARY KEY REFERENCES account (id) ON DELETE CASCADE,
    gems      BIGINT NOT NULL CHECK (gems BETWEEN 0 AND 4294967295),
    capacity  INT NOT NULL CHECK (capacity BETWEEN 16 AND 396),
    -- One item id per equipment slot (docs/knowledge/03), 0 = empty.
    equipped  INT[] NOT NULL CHECK (cardinality(equipped) = 8),
    level     INT NOT NULL DEFAULT 1 CHECK (level BETWEEN 1 AND 125),
    xp        BIGINT NOT NULL DEFAULT 0 CHECK (xp >= 0),
    saved_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS inventory (
    player_id BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    item_id   INT NOT NULL CHECK (item_id BETWEEN 1 AND 65535),
    count     INT NOT NULL CHECK (count BETWEEN 1 AND 200),
    -- Hotbar order.
    slot      INT NOT NULL CHECK (slot >= 0),
    PRIMARY KEY (player_id, item_id)
);
