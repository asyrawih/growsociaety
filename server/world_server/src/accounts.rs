//! Accounts (docs/knowledge/10, "Authentication"): register and log in with a name and password
//! over the game's own QUIC connection, which is already encrypted. Passwords are hashed with
//! Argon2id and never stored or logged. Production keeps accounts in PostgreSQL (`DATABASE_URL`,
//! migrations run at startup); without a database the server keeps them in memory for development
//! and also accepts `Auth::Dev`.

use std::sync::Mutex;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use game_protocol::{valid_name, valid_password};
use game_shared::commands::Role;
use game_shared::rules::PlayerId;

/// Who logged in. `id` is the account's, stable across sessions.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub id: PlayerId,
    pub name: String,
    pub role: Role,
}

#[derive(Debug, PartialEq)]
pub enum AuthError {
    BadName,
    BadPassword,
    NameTaken,
    /// Unknown name or wrong password: one answer, so names cannot be probed.
    WrongLogin,
    Unavailable,
}

impl AuthError {
    pub fn message(&self) -> &'static str {
        match self {
            AuthError::BadName => "Names are 1-16 letters, digits or _",
            AuthError::BadPassword => "Passwords are 8-128 characters",
            AuthError::NameTaken => "That name is taken",
            AuthError::WrongLogin => "Wrong name or password",
            AuthError::Unavailable => "Accounts are unavailable, try again later",
        }
    }
}

pub struct Stored {
    id: u32,
    name: String,
    hash: String,
    role: Role,
}

pub enum Accounts {
    /// Development: lost on restart.
    Memory(Mutex<Vec<Stored>>),
    Postgres(sqlx::PgPool),
}

/// Hashed once and verified against when a name is unknown, so a miss costs as much as a hit.
const DUMMY_PASSWORD: &str = "not a real password";

fn role_from(text: &str) -> Role {
    match text {
        "admin" => Role::Admin,
        "moderator" => Role::Moderator,
        _ => Role::Player,
    }
}

/// Argon2id with a random salt, off the async threads (it is deliberately slow).
async fn hash(password: String) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
    })
    .await
    .ok()
    .and_then(Result::ok)
    .ok_or(AuthError::Unavailable)
}

async fn verify(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false)
}

impl Accounts {
    /// PostgreSQL when `url` is given (running pending migrations), else in memory.
    pub async fn open(
        url: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let Some(url) = url.filter(|url| !url.is_empty()) else {
            return Ok(Accounts::Memory(Mutex::new(Vec::new())));
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Accounts::Postgres(pool))
    }

    /// Creates the demo accounts `demo1`..`demoN` (password = name) that do not exist yet.
    /// ponytail: for the demo phase before registration is public; remove with the lobby's register.
    pub async fn seed_demo(&self, count: u32) -> Result<(), AuthError> {
        for n in 1..=count {
            let name = format!("demo{n}");
            if self.exists(&name).await? {
                continue;
            }
            let hashed = hash(name.clone()).await?;
            self.insert(&name, hashed).await?;
        }
        Ok(())
    }

    async fn exists(&self, name: &str) -> Result<bool, AuthError> {
        match self {
            Accounts::Memory(store) => Ok(store
                .lock()
                .map_err(|_| AuthError::Unavailable)?
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(name))),
            Accounts::Postgres(pool) => {
                let row: Option<(i64,)> =
                    sqlx::query_as("SELECT id FROM account WHERE lower(name) = lower($1)")
                        .bind(name)
                        .fetch_optional(pool)
                        .await
                        .map_err(|_| AuthError::Unavailable)?;
                Ok(row.is_some())
            }
        }
    }

    /// The account called `name`, ignoring case.
    pub async fn find_name(&self, name: &str) -> Option<(PlayerId, String)> {
        if !game_protocol::valid_name(name) {
            return None;
        }
        match self {
            Accounts::Memory(store) => store.lock().ok()?.iter().find_map(|a| {
                a.name
                    .eq_ignore_ascii_case(name)
                    .then(|| (PlayerId(a.id), a.name.clone()))
            }),
            Accounts::Postgres(pool) => {
                let row: Option<(i64, String)> =
                    sqlx::query_as("SELECT id, name FROM account WHERE lower(name) = lower($1)")
                        .bind(name)
                        .fetch_optional(pool)
                        .await
                        .ok()?;
                let (id, name) = row?;
                Some((PlayerId(u32::try_from(id).ok()?), name))
            }
        }
    }

    /// Account names for `ids`; unknown ids are left out.
    pub async fn names_of(&self, ids: &[PlayerId]) -> Vec<(PlayerId, String)> {
        match self {
            Accounts::Memory(store) => store.lock().map_or_else(
                |_| Vec::new(),
                |store| {
                    store
                        .iter()
                        .filter(|a| ids.contains(&PlayerId(a.id)))
                        .map(|a| (PlayerId(a.id), a.name.clone()))
                        .collect()
                },
            ),
            Accounts::Postgres(pool) => {
                let wanted: Vec<i64> = ids.iter().map(|id| i64::from(id.0)).collect();
                let rows: Vec<(i64, String)> =
                    sqlx::query_as("SELECT id, name FROM account WHERE id = ANY($1)")
                        .bind(&wanted)
                        .fetch_all(pool)
                        .await
                        .unwrap_or_default();
                rows.into_iter()
                    .filter_map(|(id, name)| Some((PlayerId(u32::try_from(id).ok()?), name)))
                    .collect()
            }
        }
    }

    /// Development logins without an account are only for servers without a database.
    pub fn allows_dev(&self) -> bool {
        matches!(self, Accounts::Memory(_))
    }

    pub async fn register(&self, name: &str, password: &str) -> Result<Account, AuthError> {
        if !valid_name(name) {
            return Err(AuthError::BadName);
        }
        if !valid_password(password) {
            return Err(AuthError::BadPassword);
        }
        let hashed = hash(password.to_owned()).await?;
        self.insert(name, hashed).await
    }

    /// Stores a new account with an already hashed password.
    async fn insert(&self, name: &str, hashed: String) -> Result<Account, AuthError> {
        match self {
            Accounts::Memory(store) => {
                let mut store = store.lock().map_err(|_| AuthError::Unavailable)?;
                if store.iter().any(|a| a.name.eq_ignore_ascii_case(name)) {
                    return Err(AuthError::NameTaken);
                }
                let id = u32::try_from(store.len() + 1).map_err(|_| AuthError::Unavailable)?;
                store.push(Stored {
                    id,
                    name: name.to_owned(),
                    hash: hashed,
                    role: Role::Player,
                });
                Ok(Account {
                    id: PlayerId(id),
                    name: name.to_owned(),
                    role: Role::Player,
                })
            }
            Accounts::Postgres(pool) => {
                let inserted: Result<(i64,), _> = sqlx::query_as(
                    "INSERT INTO account (name, password_hash) VALUES ($1, $2) RETURNING id",
                )
                .bind(name)
                .bind(&hashed)
                .fetch_one(pool)
                .await;
                match inserted {
                    Ok((id,)) => Ok(Account {
                        id: PlayerId(u32::try_from(id).map_err(|_| AuthError::Unavailable)?),
                        name: name.to_owned(),
                        role: Role::Player,
                    }),
                    Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                        Err(AuthError::NameTaken)
                    }
                    Err(e) => {
                        eprintln!("register: {e}");
                        Err(AuthError::Unavailable)
                    }
                }
            }
        }
    }

    pub async fn login(&self, name: &str, password: &str) -> Result<Account, AuthError> {
        if !valid_name(name) || password.len() > game_protocol::MAX_PASSWORD_LEN {
            return Err(AuthError::WrongLogin);
        }
        let found: Option<(u32, String, String, Role)> = match self {
            Accounts::Memory(store) => {
                let store = store.lock().map_err(|_| AuthError::Unavailable)?;
                store
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(name))
                    .map(|a| (a.id, a.name.clone(), a.hash.clone(), a.role))
            }
            Accounts::Postgres(pool) => {
                let row: Option<(i64, String, String, String)> = sqlx::query_as(
                    "SELECT id, name, password_hash, role FROM account WHERE lower(name) = lower($1)",
                )
                .bind(name)
                .fetch_optional(pool)
                .await
                .map_err(|e| {
                    eprintln!("login: {e}");
                    AuthError::Unavailable
                })?;
                match row {
                    Some((id, name, hash, role)) => Some((
                        u32::try_from(id).map_err(|_| AuthError::Unavailable)?,
                        name,
                        hash,
                        role_from(&role),
                    )),
                    None => None,
                }
            }
        };
        let Some((id, name, stored, role)) = found else {
            // Same work as a real check, so timing does not reveal which names exist.
            let dummy = hash(DUMMY_PASSWORD.to_owned()).await?;
            verify(password.to_owned(), dummy).await;
            return Err(AuthError::WrongLogin);
        };
        if verify(password.to_owned(), stored).await {
            Ok(Account {
                id: PlayerId(id),
                name,
                role,
            })
        } else {
            Err(AuthError::WrongLogin)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_then_login_in_memory() {
        let accounts = Accounts::open(None).await.expect("memory");
        assert!(accounts.allows_dev());
        let made = accounts
            .register("Budi", "hunter4242")
            .await
            .expect("registered");
        assert_eq!(made.name, "Budi");
        assert_eq!(made.role, Role::Player);

        assert_eq!(
            accounts.register("budi", "anotherpass").await,
            Err(AuthError::NameTaken)
        );
        assert_eq!(
            accounts.register("a b", "hunter4242").await,
            Err(AuthError::BadName)
        );
        assert_eq!(
            accounts.register("Cici", "short").await,
            Err(AuthError::BadPassword)
        );

        let back = accounts
            .login("BUDI", "hunter4242")
            .await
            .expect("logged in");
        assert_eq!(back.id, made.id, "the same account across sessions");
        assert_eq!(
            accounts.login("Budi", "wrong-pass").await,
            Err(AuthError::WrongLogin)
        );
        assert_eq!(
            accounts.login("Nobody", "hunter4242").await,
            Err(AuthError::WrongLogin)
        );
    }

    #[tokio::test]
    async fn demo_accounts_are_seeded_once() {
        let accounts = Accounts::open(None).await.expect("memory");
        accounts.seed_demo(3).await.expect("seeded");
        accounts
            .seed_demo(3)
            .await
            .expect("seeding again changes nothing");
        let demo2 = accounts.login("demo2", "demo2").await.expect("demo login");
        assert_eq!(demo2.name, "demo2");
        assert_eq!(
            accounts.login("demo4", "demo4").await,
            Err(AuthError::WrongLogin)
        );
        assert_eq!(
            accounts.login("demo1", "demo2").await,
            Err(AuthError::WrongLogin)
        );
    }

    #[tokio::test]
    async fn passwords_are_stored_as_argon2id_hashes() {
        let stored = hash("hunter4242".into()).await.expect("hashed");
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert!(!stored.contains("hunter4242"));
        assert!(verify("hunter4242".into(), stored.clone()).await);
        assert!(!verify("hunter4243".into(), stored).await);
    }
}
