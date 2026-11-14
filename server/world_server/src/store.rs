//! Persistence (docs/knowledge/10, "Persistence"). Worlds are a versioned blob; inventories, gems,
//! equipment and level are rows. The world task saves its world together with the players in it,
//! in one transaction, on a timer, when someone leaves, after a trade, and on unload. Crash
//! recovery is therefore "everything as of the last save", never an item in two places.
//! Without a database (development) nothing is saved and worlds are generated fresh.

use game_shared::inventory::Inventory;
use game_shared::items::{ItemId, SLOT_COUNT};
use game_shared::rng::Rng;
use game_shared::rules::{Player, PlayerId, WorldState};
use game_shared::world::World;
use serde::{Deserialize, Serialize};

use crate::accounts::Accounts;
use crate::conn::DEV_ID_BASE;

/// The first two bytes of every world blob. Bump on an incompatible change and add a branch to
/// `decode_world` that migrates the old layout (item ids never change meaning, so old worlds stay valid).
pub const WORLD_FORMAT: u16 = 1;

#[derive(Serialize)]
struct WorldBlobRef<'a> {
    world: &'a World,
    rng: &'a Rng,
}

#[derive(Deserialize)]
struct WorldBlob {
    world: World,
    rng: Rng,
}

/// Tiles, extras, drops and the world's RNG state. Players, damage and open trades are not part
/// of a world save: players have their own rows, damage expires and trades restart.
pub fn encode_world(state: &WorldState) -> Vec<u8> {
    let mut bytes = WORLD_FORMAT.to_le_bytes().to_vec();
    let blob = WorldBlobRef {
        world: &state.world,
        rng: &state.rng,
    };
    bytes.extend(game_protocol::encode(&blob));
    bytes
}

pub fn decode_world(bytes: &[u8]) -> Option<WorldState> {
    let (version, payload) = bytes.split_first_chunk::<2>()?;
    let blob: WorldBlob = match u16::from_le_bytes(*version) {
        WORLD_FORMAT => game_protocol::decode(payload).ok()?,
        _ => return None,
    };
    let mut world = blob.world;
    // Derived, never stored.
    (world.claim_of, world.world_claim) = world.claim_index_from_extras();
    let mut state = WorldState::new(world, 0);
    state.rng = blob.rng;
    Some(state)
}

/// One player's saved rows.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSave {
    pub id: PlayerId,
    pub gems: u32,
    pub capacity: u16,
    /// In hotbar order.
    pub items: Vec<(ItemId, u16)>,
    pub equipped: [Option<ItemId>; SLOT_COUNT],
    pub level: u16,
    pub xp: u32,
    /// Notices of claims a reset returned while the player was elsewhere. `load_player` has
    /// already paid them, in the database; the world only shows these once the player is in.
    pub notices: Vec<String>,
    /// Who this player has muted with `/ignore`, in every world.
    pub ignores: Vec<PlayerId>,
}

/// One claim returned by `/resetworld` to an owner who was not in that world (`returned_claim`).
#[derive(Clone, Debug, PartialEq)]
pub struct Returned {
    pub player: PlayerId,
    pub item: ItemId,
    /// Paid in gems instead when the backpack has no room.
    pub price: u32,
    pub notice: String,
}

/// One report to store (`reports`).
pub struct Report<'a> {
    pub reporter: PlayerId,
    pub target: PlayerId,
    pub world: &'a str,
    pub reason: &'static str,
    pub note: Option<&'a str>,
    /// JSON array of the context lines.
    pub chat: String,
}

/// What a world save writes besides the world and its players.
#[derive(Default)]
pub struct SaveExtras<'a> {
    /// The world before a reset (`world_backup`), written before the new world.
    pub backup: Option<&'a [u8]>,
    /// New `returned_claim` rows.
    pub returned: &'a [Returned],
    /// The world is stored for the first time: count it towards this account's creation limit.
    pub created_by: Option<PlayerId>,
}

impl PlayerSave {
    pub fn of(player: &Player) -> Self {
        PlayerSave {
            id: player.id,
            gems: player.gems,
            capacity: player.inventory.capacity,
            items: player.inventory.slots().to_vec(),
            equipped: player.equipped,
            level: player.level,
            xp: player.xp,
            notices: Vec::new(),
            ignores: Vec::new(),
        }
    }

    /// Restores the saved state onto a freshly joined player (who has the starter kit).
    pub fn apply_to(&self, player: &mut Player) {
        let mut inventory = Inventory::default();
        inventory.capacity = self.capacity;
        for &(item, count) in &self.items {
            let held = inventory.count(item);
            if count > held {
                inventory.add(item, count - held);
            }
        }
        player.inventory = inventory;
        player.gems = self.gems;
        // Only what is still in the inventory can be worn.
        player.equipped = self
            .equipped
            .map(|worn| worn.filter(|&item| player.inventory.count(item) > 0));
        player.level = self.level.max(1);
        player.xp = self.xp;
    }
}

/// Accounts with rows; development players have none.
fn persistent(id: PlayerId) -> bool {
    id.0 < DEV_ID_BASE
}

impl Accounts {
    pub async fn load_world(&self, name: &str) -> Option<WorldState> {
        let Accounts::Postgres(pool) = self else {
            return None;
        };
        let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT data FROM world WHERE name = $1")
            .bind(name)
            .fetch_optional(pool)
            .await
            .map_err(|e| eprintln!("load world {name}: {e}"))
            .ok()?;
        let state = decode_world(&row?.0);
        if state.is_none() {
            eprintln!("world {name}: unreadable save, generating a new one");
        }
        state
    }

    /// Stores a report. `chat` is the JSON array of context lines.
    pub async fn store_report(&self, report: &Report<'_>) -> Result<(), String> {
        let Accounts::Postgres(pool) = self else {
            return Ok(());
        };
        if !persistent(report.reporter) {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO reports (reporter_id, target_id, world, reason, note, chat)
             VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
        )
        .bind(i64::from(report.reporter.0))
        .bind(i64::from(report.target.0))
        .bind(report.world)
        .bind(report.reason)
        .bind(report.note)
        .bind(&report.chat)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Reports `id` sent in the last hour.
    pub async fn reports_last_hour(&self, id: PlayerId) -> u32 {
        let Accounts::Postgres(pool) = self else {
            return 0;
        };
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM reports WHERE reporter_id = $1 AND created_at > now() - interval '1 hour'",
        )
        .bind(i64::from(id.0))
        .fetch_one(pool)
        .await
        .map_err(|e| eprintln!("reports {}: {e}", id.0))
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Whether `name` is stored (it was built in at least once).
    pub async fn world_exists(&self, name: &str) -> bool {
        let Accounts::Postgres(pool) = self else {
            return false;
        };
        sqlx::query_scalar::<_, i32>("SELECT 1 FROM world WHERE name = $1")
            .bind(name)
            .fetch_optional(pool)
            .await
            .map_err(|e| eprintln!("world exists {name}: {e}"))
            .is_ok_and(|row| row.is_some())
    }

    /// Worlds `id` opened in the last hour (`world_created`).
    pub async fn worlds_opened_last_hour(&self, id: PlayerId) -> u32 {
        let Accounts::Postgres(pool) = self else {
            return 0;
        };
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM world_created
             WHERE player_id = $1 AND created_at > now() - interval '1 hour'",
        )
        .bind(i64::from(id.0))
        .fetch_one(pool)
        .await
        .map_err(|e| eprintln!("worlds opened {}: {e}", id.0))
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
    }

    pub async fn load_player(&self, id: PlayerId) -> Option<PlayerSave> {
        let Accounts::Postgres(pool) = self else {
            return None;
        };
        if !persistent(id) {
            return None;
        }
        let state: (i64, i32, Vec<i32>, i32, i64) = sqlx::query_as(
            "SELECT gems, capacity, equipped, level, xp FROM player_state WHERE player_id = $1",
        )
        .bind(i64::from(id.0))
        .fetch_optional(pool)
        .await
        .map_err(|e| eprintln!("load player {}: {e}", id.0))
        .ok()??;
        let rows: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT item_id, count FROM inventory WHERE player_id = $1 ORDER BY slot",
        )
        .bind(i64::from(id.0))
        .fetch_all(pool)
        .await
        .map_err(|e| eprintln!("load inventory {}: {e}", id.0))
        .ok()?;
        let ignores: Vec<(i64,)> =
            sqlx::query_as("SELECT ignored_id FROM ignores WHERE player_id = $1")
                .bind(i64::from(id.0))
                .fetch_all(pool)
                .await
                .map_err(|e| eprintln!("load ignores {}: {e}", id.0))
                .unwrap_or_default();
        let ignores = ignores
            .into_iter()
            .filter_map(|(n,)| u32::try_from(n).ok().map(PlayerId))
            .collect();
        let (gems, capacity, worn, level, xp) = state;
        let item = |n: i32| u16::try_from(n).ok().filter(|&n| n > 0).map(ItemId);
        let mut equipped = [None; SLOT_COUNT];
        for (slot, &id) in equipped.iter_mut().zip(&worn) {
            *slot = item(id);
        }
        let save = PlayerSave {
            id,
            gems: u32::try_from(gems).unwrap_or(0),
            capacity: u16::try_from(capacity).unwrap_or(Inventory::default().capacity),
            items: rows
                .into_iter()
                .filter_map(|(id, count)| Some((item(id)?, u16::try_from(count).ok()?)))
                .collect(),
            equipped,
            level: u16::try_from(level).unwrap_or(1),
            xp: u32::try_from(xp).unwrap_or(0),
            notices: Vec::new(),
            ignores,
        };
        Some(self.pay_returned(pool, save).await)
    }

    /// Pays the claims a reset returned to this player, in one transaction: the rows are deleted
    /// and the credited inventory and gems written together. Either both happen or neither, so a
    /// claim is never paid twice (and never lost); on any error the rows wait for the next load.
    async fn pay_returned(&self, pool: &sqlx::PgPool, save: PlayerSave) -> PlayerSave {
        let paid = async {
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            let rows: Vec<(i32, i64, String)> = sqlx::query_as(
                "DELETE FROM returned_claim WHERE player_id = $1 RETURNING item_id, price, notice",
            )
            .bind(i64::from(save.id.0))
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            if rows.is_empty() {
                return Ok::<_, String>(None);
            }
            let rows: Vec<Returned> = rows
                .into_iter()
                .filter_map(|(item_id, price, notice)| {
                    Some(Returned {
                        player: save.id,
                        item: ItemId(u16::try_from(item_id).ok()?),
                        price: u32::try_from(price).unwrap_or(u32::MAX),
                        notice,
                    })
                })
                .collect();
            let mut credited = save.clone();
            credit(&mut credited, &rows);
            write_player(&mut tx, &credited).await?;
            tx.commit().await.map_err(|e| e.to_string())?;
            Ok(Some(credited))
        };
        match paid.await {
            Ok(Some(credited)) => credited,
            Ok(None) => save,
            Err(error) => {
                eprintln!("pay returned claims {}: {error}", save.id.0);
                save
            }
        }
    }

    /// The world and its players (and `extras`), all or nothing.
    pub async fn save(
        &self,
        name: &str,
        world: Option<&[u8]>,
        players: &[PlayerSave],
        extras: SaveExtras<'_>,
    ) -> Result<(), String> {
        let Accounts::Postgres(pool) = self else {
            return Ok(());
        };
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        if let Some(backup) = extras.backup {
            sqlx::query(
                "INSERT INTO world_backup (name, data, saved_at) SELECT name, data, now()
                 FROM world WHERE name = $1
                 ON CONFLICT (name) DO UPDATE SET data = EXCLUDED.data, saved_at = now()",
            )
            .bind(name)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            // A world never saved before has no row to copy: back up the blob in memory.
            sqlx::query(
                "INSERT INTO world_backup (name, data) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
            )
            .bind(name)
            .bind(backup)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        for row in extras.returned.iter().filter(|r| persistent(r.player)) {
            sqlx::query(
                "INSERT INTO returned_claim (player_id, item_id, price, notice) VALUES ($1, $2, $3, $4)",
            )
            .bind(i64::from(row.player.0))
            .bind(i32::from(row.item.0))
            .bind(i64::from(row.price))
            .bind(&row.notice)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        if let Some(world) = world {
            sqlx::query(
                "INSERT INTO world (name, data, saved_at) VALUES ($1, $2, now())
                 ON CONFLICT (name) DO UPDATE SET data = $2, saved_at = now()",
            )
            .bind(name)
            .bind(world)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        if let Some(by) = extras.created_by.filter(|&by| persistent(by)) {
            sqlx::query(
                "INSERT INTO world_created (name, player_id) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
            )
            .bind(name)
            .bind(i64::from(by.0))
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        for player in players.iter().filter(|p| persistent(p.id)) {
            write_player(&mut tx, player).await?;
        }
        tx.commit().await.map_err(|e| e.to_string())
    }
}

/// One player's gems, equipment, level and inventory rows, inside a save's transaction.
async fn write_player(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    player: &PlayerSave,
) -> Result<(), String> {
    let id = i64::from(player.id.0);
    let worn: Vec<i32> = player
        .equipped
        .iter()
        .map(|slot| slot.map_or(0, |item| i32::from(item.0)))
        .collect();
    sqlx::query(
        "INSERT INTO player_state (player_id, gems, capacity, equipped, level, xp, saved_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT (player_id) DO UPDATE SET gems = $2, capacity = $3, equipped = $4,
                 level = $5, xp = $6, saved_at = now()",
    )
    .bind(id)
    .bind(i64::from(player.gems))
    .bind(i32::from(player.capacity))
    .bind(&worn)
    .bind(i32::from(player.level))
    .bind(i64::from(player.xp))
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM inventory WHERE player_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let items: Vec<i32> = player.items.iter().map(|(i, _)| i32::from(i.0)).collect();
    let counts: Vec<i32> = player.items.iter().map(|&(_, c)| i32::from(c)).collect();
    let slots: Vec<i32> = (0..).take(items.len()).collect();
    sqlx::query(
        "INSERT INTO inventory (player_id, item_id, count, slot)
             SELECT $1, * FROM UNNEST($2::INT[], $3::INT[], $4::INT[])",
    )
    .bind(id)
    .bind(&items)
    .bind(&counts)
    .bind(&slots)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    // The `/ignore` list, replaced whole like the inventory.
    sqlx::query("DELETE FROM ignores WHERE player_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let ignored: Vec<i64> = player.ignores.iter().map(|p| i64::from(p.0)).collect();
    sqlx::query(
        "INSERT INTO ignores (player_id, ignored_id) SELECT $1, * FROM UNNEST($2::BIGINT[])",
    )
    .bind(id)
    .bind(&ignored)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Adds returned claims to a saved player: the item where the backpack has room, else its store
/// price in gems. The same rule as `Room::pay` for owners who are online.
fn credit(save: &mut PlayerSave, rows: &[Returned]) {
    // Rebuilt like `apply_to`: every save already holds the permanent fist and wrench.
    let mut inventory = Inventory::default();
    inventory.capacity = save.capacity;
    for &(item, count) in &save.items {
        let held = inventory.count(item);
        inventory.add(item, count.saturating_sub(held));
    }
    for row in rows {
        if inventory.add(row.item, 1) == 0 {
            save.gems = save.gems.saturating_add(row.price);
        }
        save.notices.push(row.notice.clone());
    }
    save.items = inventory.slots().to_vec();
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::claims::Claim;
    use game_shared::items::{DIRT, SMALL_CLAIM};
    use game_shared::worldgen::generate;

    #[test]
    fn a_world_survives_a_save_and_load() {
        let mut state = WorldState::new(generate("SAVE"), 77);
        state.rng.next_f32();
        let claim = Claim {
            owner: PlayerId(3),
            access: vec![],
            public: false,
            tiles: vec![10, 11],
        };
        state.world.insert_claim(10, claim);
        let blob = encode_world(&state);
        assert_eq!(&blob[..2], &WORLD_FORMAT.to_le_bytes());
        let back = decode_world(&blob).expect("decodes");
        assert_eq!(
            back.world, state.world,
            "tiles, extras, drops and the rebuilt claim index"
        );
        assert_eq!(back.rng, state.rng, "the RNG continues where it stopped");
        assert!(
            decode_world(&[9, 9, 1, 2]).is_none(),
            "an unknown version is refused"
        );
        assert!(decode_world(&[]).is_none());
    }

    #[test]
    fn a_player_save_restores_inventory_gems_and_equipment() {
        let mut state = WorldState::new(generate("SAVE"), 1);
        let id = PlayerId(5);
        state.join(id);
        let player = state.players.get_mut(&id).expect("joined");
        player.inventory.add(DIRT, 37);
        player.inventory.add(SMALL_CLAIM, 2);
        player.gems = 1234;
        player.level = 4;
        player.xp = 99;
        let saved = PlayerSave::of(player);

        let mut fresh = WorldState::new(generate("OTHER"), 1);
        fresh.join(id);
        let restored = fresh.players.get_mut(&id).expect("joined");
        saved.apply_to(restored);
        assert_eq!(PlayerSave::of(restored), saved);
        assert_eq!(
            restored.inventory, state.players[&id].inventory,
            "same items, same order"
        );
    }

    #[test]
    fn credit_adds_the_claim_or_its_price_when_full() {
        let id = PlayerId(5);
        let row = |item| Returned {
            player: id,
            item,
            price: 50,
            notice: "returned".into(),
        };
        let mut save = PlayerSave {
            id,
            gems: 10,
            capacity: Inventory::default().capacity,
            items: [Inventory::default().slots(), &[(DIRT, 3)]].concat(),
            equipped: [None; SLOT_COUNT],
            level: 1,
            xp: 0,
            notices: vec![],
            ignores: vec![],
        };
        let before = save.items.clone();
        credit(&mut save, &[row(SMALL_CLAIM)]);
        assert_eq!(
            save.items,
            [before.as_slice(), &[(SMALL_CLAIM, 1)]].concat()
        );
        assert_eq!((save.gems, save.notices.len()), (10, 1));

        // No room: every slot holds a full stack of something else.
        let mut full = save.clone();
        full.items = [
            Inventory::default().slots(),
            &[(DIRT, 200), (ItemId(2), 200)],
        ]
        .concat();
        full.capacity = full.items.len() as u16;
        let packed = full.items.clone();
        credit(&mut full, &[row(SMALL_CLAIM)]);
        assert_eq!(full.items, packed);
        assert_eq!(full.gems, 60, "paid the store price instead");
    }

    /// Against a real PostgreSQL: the ignore list survives a save and load, and a report is stored
    /// and counted for the hourly limit.
    #[tokio::test]
    #[ignore]
    async fn postgres_ignores_and_reports() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let db = Accounts::open(Some(url))
            .await
            .expect("connects and migrates");
        let account = db
            .register(&format!("i{}", std::process::id()), "password1")
            .await
            .expect("account");
        let mut state = WorldState::new(generate("PGIGNORE"), 9);
        state.join(account.id);
        let mut saved = PlayerSave::of(&state.players[&account.id]);
        saved.ignores = vec![PlayerId(7), PlayerId(9)];
        db.save(
            "PGIGNORE",
            None,
            std::slice::from_ref(&saved),
            SaveExtras::default(),
        )
        .await
        .expect("saved");
        let loaded = db.load_player(account.id).await.expect("loads");
        let mut ignores = loaded.ignores.clone();
        ignores.sort_by_key(|p| p.0);
        assert_eq!(ignores, [PlayerId(7), PlayerId(9)]);

        assert_eq!(db.reports_last_hour(account.id).await, 0);
        let report = Report {
            reporter: account.id,
            target: PlayerId(7),
            world: "PGIGNORE",
            reason: "spam",
            note: Some("selling"),
            chat: r#"[{"from":7,"text":"buy","at":1.0}]"#.into(),
        };
        db.store_report(&report).await.expect("stored");
        assert_eq!(db.reports_last_hour(account.id).await, 1);
    }

    /// Against a real PostgreSQL: a returned claim is paid once. The first load deletes the row
    /// and writes the credit in one transaction; a later load (as after a failed world save) finds
    /// no row and the credit already stored, so nothing is paid twice.
    /// `TEST_DATABASE_URL=postgres://… cargo test -p world_server -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn postgres_returned_claim_is_paid_once() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let db = Accounts::open(Some(url))
            .await
            .expect("connects and migrates");
        let name = format!("r{}", std::process::id());
        let account = db.register(&name, "password1").await.expect("account");
        let mut state = WorldState::new(generate("PGRESET"), 9);
        state.join(account.id);
        let saved = PlayerSave::of(&state.players[&account.id]);
        let returned = Returned {
            player: account.id,
            item: SMALL_CLAIM,
            price: 50,
            notice: "returned".into(),
        };
        let extras = SaveExtras {
            returned: std::slice::from_ref(&returned),
            ..SaveExtras::default()
        };
        db.save(
            "PGRESET",
            Some(&encode_world(&state)),
            std::slice::from_ref(&saved),
            extras,
        )
        .await
        .expect("saved");

        let first = db.load_player(account.id).await.expect("loads");
        let claims = |save: &PlayerSave| {
            save.items
                .iter()
                .find(|(item, _)| *item == SMALL_CLAIM)
                .map_or(0, |&(_, n)| n)
        };
        let before = claims(&saved);
        assert_eq!(claims(&first), before + 1);
        assert_eq!(first.notices, ["returned"]);

        // The world never saved this player again (its save failed): load once more.
        let second = db.load_player(account.id).await.expect("loads");
        assert_eq!(claims(&second), before + 1, "paid exactly once");
        assert!(second.notices.is_empty());
    }

    /// Against a real PostgreSQL: `TEST_DATABASE_URL=postgres://… cargo test -p world_server -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn postgres_round_trip() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let db = Accounts::open(Some(url))
            .await
            .expect("connects and migrates");
        let name = format!("t{}", std::process::id());
        let account = db.register(&name, "password1").await.expect("account");

        let mut state = WorldState::new(generate("PGTEST"), 9);
        state.join(account.id);
        let player = state.players.get_mut(&account.id).expect("joined");
        player.inventory.add(DIRT, 150);
        player.gems = 42;
        let saved = PlayerSave::of(player);
        db.save(
            "PGTEST",
            Some(&encode_world(&state)),
            std::slice::from_ref(&saved),
            SaveExtras::default(),
        )
        .await
        .expect("saved");

        assert_eq!(db.load_player(account.id).await, Some(saved.clone()));
        assert_eq!(
            db.load_world("PGTEST").await.map(|s| s.world),
            Some(state.world.clone())
        );

        // The database refuses what the rules refuse: 201 of an item rolls the whole save back.
        let mut bad = saved.clone();
        bad.gems = 7;
        bad.items.push((ItemId(2), 201));
        assert!(
            db.save(
                "PGTEST",
                Some(&encode_world(&state)),
                &[bad],
                SaveExtras::default()
            )
            .await
            .is_err()
        );
        assert_eq!(
            db.load_player(account.id).await.map(|p| p.gems),
            Some(42),
            "rolled back"
        );
    }
}
