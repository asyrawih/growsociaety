-- Player reports and persistent mutes (docs/knowledge/ux/avatar-menu.md).

-- One row per report. `chat` holds the last chat lines the reporter received in that world, as
-- [{"from": id, "text": "...", "at": seconds}], for staff to judge; there is no staff UI yet.
CREATE TABLE IF NOT EXISTS reports (
    id          BIGSERIAL PRIMARY KEY,
    reporter_id BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    target_id   BIGINT NOT NULL,
    world       TEXT NOT NULL,
    reason      TEXT NOT NULL CHECK (reason IN ('spam', 'rude', 'cheating')),
    note        TEXT,
    chat        JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS reports_by_reporter ON reports (reporter_id, created_at);

-- `/ignore` lists, kept across logins and applied in every world. Saved with the player.
CREATE TABLE IF NOT EXISTS ignores (
    player_id  BIGINT NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    ignored_id BIGINT NOT NULL,
    PRIMARY KEY (player_id, ignored_id)
);
