-- World Deeds (docs/knowledge/ux/world-key.md). The row IS the deed: `holder_id` is whose ring it
-- is on. At most one valid deed per world (the partial unique index); a void deed stays until its
-- holder discards it. Using a deed deletes its row only if the user still holds it, so a deed can
-- never be used twice.
CREATE TABLE IF NOT EXISTS world_deeds (
    id        BIGSERIAL PRIMARY KEY,
    world     TEXT NOT NULL,
    holder_id BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    void      BOOLEAN NOT NULL DEFAULT false,
    made_by   BIGINT NOT NULL,
    made_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS one_valid_deed_per_world ON world_deeds (world) WHERE NOT void;
CREATE INDEX IF NOT EXISTS world_deeds_by_holder ON world_deeds (holder_id);

-- Private lines for players who were offline when something happened to them (a deed for their
-- world was used). Shown, and deleted, at their next login.
CREATE TABLE IF NOT EXISTS player_notices (
    id        BIGSERIAL PRIMARY KEY,
    player_id BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    notice    TEXT NOT NULL
);
