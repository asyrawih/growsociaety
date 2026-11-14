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
        let (gems, capacity, worn, level, xp) = state;
        let item = |n: i32| u16::try_from(n).ok().filter(|&n| n > 0).map(ItemId);
        let mut equipped = [None; SLOT_COUNT];
        for (slot, &id) in equipped.iter_mut().zip(&worn) {
            *slot = item(id);
        }
        Some(PlayerSave {
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
        })
    }

    /// The world and its players, all or nothing.
    pub async fn save(
        &self,
        name: &str,
        world: &[u8],
        players: &[PlayerSave],
    ) -> Result<(), String> {
        let Accounts::Postgres(pool) = self else {
            return Ok(());
        };
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO world (name, data, saved_at) VALUES ($1, $2, now())
             ON CONFLICT (name) DO UPDATE SET data = $2, saved_at = now()",
        )
        .bind(name)
        .bind(world)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        for player in players.iter().filter(|p| persistent(p.id)) {
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
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM inventory WHERE player_id = $1")
                .bind(id)
                .execute(&mut *tx)
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
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())
    }
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
            &encode_world(&state),
            std::slice::from_ref(&saved),
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
            db.save("PGTEST", &encode_world(&state), &[bad])
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
