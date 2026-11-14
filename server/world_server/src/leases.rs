//! Multi-server ownership (docs/knowledge/10, "Scaling"; tables in migrations/0001_leases.sql).
//! An account is online on one server at a time and a world runs on one server at a time, or items
//! could be duplicated. Leases expire 30 s after the last renewal, so a crashed server's worlds and
//! players free themselves; a live server renews everything it holds every 10 s.
//! Without a database there is one server and nothing to share: every claim succeeds.

use std::time::Duration;

use game_shared::rules::PlayerId;

use crate::accounts::Accounts;

/// How often a server renews its leases and reports its load. Reference (doc 10).
pub const RENEW_EVERY: Duration = Duration::from_secs(10);

/// Who this server is to the others: `GAME_SERVER_ID` (else a random id) and `GAME_PUBLIC_ADDR`,
/// the `host:port` clients are sent to for worlds it holds.
#[derive(Clone, Debug)]
pub struct Identity {
    pub id: String,
    pub addr: String,
}

impl Identity {
    pub fn from_env() -> Self {
        let id = std::env::var("GAME_SERVER_ID").unwrap_or_else(|_| {
            let mut bytes = [0u8; 6];
            let _ = getrandom::fill(&mut bytes);
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        });
        let addr = std::env::var("GAME_PUBLIC_ADDR").unwrap_or_default();
        Identity { id, addr }
    }
}

impl Accounts {
    /// `true` if the account is now online here (new, expired elsewhere, or already ours).
    pub async fn claim_session(&self, me: &Identity, player: PlayerId) -> bool {
        let Accounts::Postgres(pool) = self else {
            return true;
        };
        let row: Result<Option<(i64,)>, _> = sqlx::query_as(
            "INSERT INTO player_session VALUES ($1, $2, now() + interval '30 seconds')
             ON CONFLICT (player_id) DO UPDATE
               SET server_id = $2, expires_at = now() + interval '30 seconds'
               WHERE player_session.expires_at < now() OR player_session.server_id = $2
             RETURNING player_id",
        )
        .bind(i64::from(player.0))
        .bind(&me.id)
        .fetch_optional(pool)
        .await;
        row.map_err(|e| eprintln!("claim session: {e}"))
            .is_ok_and(|row| row.is_some())
    }

    pub async fn release_session(&self, me: &Identity, player: PlayerId) {
        if let Accounts::Postgres(pool) = self {
            let _ =
                sqlx::query("DELETE FROM player_session WHERE player_id = $1 AND server_id = $2")
                    .bind(i64::from(player.0))
                    .bind(&me.id)
                    .execute(pool)
                    .await;
        }
    }

    /// `None` if this server may run the world; `Some(addr)` of the server that holds it.
    pub async fn claim_world(&self, me: &Identity, world: &str) -> Option<String> {
        let Accounts::Postgres(pool) = self else {
            return None;
        };
        let ours: Result<Option<(String,)>, _> = sqlx::query_as(
            "INSERT INTO world_lease VALUES ($1, $2, $3, now() + interval '30 seconds')
             ON CONFLICT (world) DO UPDATE
               SET server_id = $2, addr = $3, expires_at = now() + interval '30 seconds'
               WHERE world_lease.expires_at < now() OR world_lease.server_id = $2
             RETURNING world",
        )
        .bind(world)
        .bind(&me.id)
        .bind(&me.addr)
        .fetch_optional(pool)
        .await;
        match ours {
            Ok(Some(_)) => None,
            Ok(None) => {
                let holder: Option<(String,)> =
                    sqlx::query_as("SELECT addr FROM world_lease WHERE world = $1")
                        .bind(world)
                        .fetch_optional(pool)
                        .await
                        .ok()
                        .flatten();
                Some(holder.map_or_else(String::new, |(addr,)| addr))
            }
            Err(e) => {
                // Cannot tell: refuse rather than risk the world running twice.
                eprintln!("claim world {world}: {e}");
                Some(String::new())
            }
        }
    }

    /// After the final save of an unloading world.
    pub async fn release_world(&self, me: &Identity, world: &str) {
        if let Accounts::Postgres(pool) = self {
            let _ = sqlx::query("DELETE FROM world_lease WHERE world = $1 AND server_id = $2")
                .bind(world)
                .bind(&me.id)
                .execute(pool)
                .await;
        }
    }

    /// Renews every lease this server holds and reports its load.
    pub async fn heartbeat(&self, me: &Identity, online: u32) {
        let Accounts::Postgres(pool) = self else {
            return;
        };
        let renew = [
            "UPDATE world_lease SET expires_at = now() + interval '30 seconds' WHERE server_id = $1",
            "UPDATE player_session SET expires_at = now() + interval '30 seconds' WHERE server_id = $1",
        ];
        for query in renew {
            if let Err(e) = sqlx::query(query).bind(&me.id).execute(pool).await {
                eprintln!("renew leases: {e}");
            }
        }
        let report = sqlx::query(
            "INSERT INTO game_server VALUES ($1, $2, $3, now() + interval '30 seconds')
             ON CONFLICT (server_id) DO UPDATE
               SET addr = $2, online = $3, expires_at = now() + interval '30 seconds'",
        )
        .bind(&me.id)
        .bind(&me.addr)
        .bind(i32::try_from(online).unwrap_or(i32::MAX))
        .execute(pool)
        .await;
        if let Err(e) = report {
            eprintln!("heartbeat: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TEST_DATABASE_URL=postgres://… cargo test -p world_server -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn leases_keep_worlds_and_accounts_on_one_server() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let db = Accounts::open(Some(url)).await.expect("connects");
        let a = Identity {
            id: "server-a".into(),
            addr: "a:7777".into(),
        };
        let b = Identity {
            id: "server-b".into(),
            addr: "b:7777".into(),
        };
        let world = format!("W{}", std::process::id());
        assert_eq!(db.claim_world(&a, &world).await, None, "first claim wins");
        assert_eq!(
            db.claim_world(&a, &world).await,
            None,
            "re-claiming your own is fine"
        );
        assert_eq!(
            db.claim_world(&b, &world).await,
            Some("a:7777".into()),
            "redirect to a"
        );
        db.release_world(&a, &world).await;
        assert_eq!(db.claim_world(&b, &world).await, None, "free after release");

        let player = PlayerId(900_000 + std::process::id() % 1000);
        assert!(db.claim_session(&a, player).await);
        assert!(!db.claim_session(&b, player).await, "online once");
        db.heartbeat(&a, 1).await;
        db.release_session(&a, player).await;
        assert!(db.claim_session(&b, player).await);
        db.release_session(&b, player).await;
    }
}
