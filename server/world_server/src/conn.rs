//! One task per connection: the boundary checks of docs/knowledge/10 ("Validation at the boundary").
//! Size before decode, protocol version at `Hello`, identity from the connection, increasing `seq`,
//! per-type rate limits. Everything past this point is `apply` / `step` inside the world task.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use game_protocol::{
    ACTIVE_WORLDS_LISTED, Auth, ClientMsg, MAX_CLIENT_MSG, MAX_LOOKUP, MAX_TOKEN_LEN,
    NEW_WORLDS_PER_HOUR, PROTOCOL_VERSION, ReadError, ServerMsg, WORLD_CAPACITY, WorldStatus,
    clean_chat, encode, read_msg, valid_country, valid_name, world_name_allowed, write_msg,
};
use game_shared::commands::Role;
use game_shared::physics::TICK_RATE;
use game_shared::rules::PlayerId;
use tokio::sync::mpsc;

use crate::accounts::Accounts;
use crate::world::{Clock, OUTBOX, Out, Registry, ToWorld, send_to};

/// Inputs per second: one per physics tick, so a client cannot move faster by sending more.
const INPUT_RATE: f32 = TICK_RATE;
/// Inputs that may arrive at once after a network hiccup.
const INPUT_BURST: f32 = 30.0;
/// Actions per second (docs/knowledge/10 example: 10).
const ACT_RATE: f32 = 10.0;
const ACT_BURST: f32 = 10.0;
/// Chat lines per second, with a short burst (docs/knowledge/10, "Text": rate limit).
const CHAT_RATE: f32 = 1.0;
const CHAT_BURST: f32 = 5.0;
/// Voice mutes per second (docs/knowledge/12: rate-limited), with a short burst.
const VOICE_MUTE_RATE: f32 = 1.0;
const VOICE_MUTE_BURST: f32 = 3.0;
/// Name lookups per second, with a burst (each is one small query).
const LOOKUP_RATE: f32 = 2.0;
const LOOKUP_BURST: f32 = 5.0;
/// Dropped messages tolerated before the connection is closed.
const MAX_STRIKES: u32 = 200;
/// Players one server process takes; more players means another server (docs/knowledge/10, "Scaling").
pub const MAX_PLAYERS: u32 = 500;

/// Online-player count with a hard cap. A taken slot is released when its guard drops,
/// so every exit path of `handle` frees it.
#[derive(Default)]
pub struct Slots(AtomicU32);

pub struct Slot<'a>(&'a AtomicU32);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Slots {
    pub fn take(&self, max: u32) -> Option<Slot<'_>> {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < max).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(&self.0))
    }

    pub fn online(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Token bucket: `rate` per second, at most `burst` saved up.
pub struct Bucket {
    tokens: f32,
    rate: f32,
    burst: f32,
    last: Instant,
}

impl Bucket {
    pub fn new(rate: f32, burst: f32, now: Instant) -> Self {
        Self {
            tokens: burst,
            rate,
            burst,
            last: now,
        }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f32();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        let ok = self.tokens >= 1.0;
        if ok {
            self.tokens -= 1.0;
        }
        ok
    }
}

/// Development ids (no account) start here, far above any account id, so the two never meet.
pub const DEV_ID_BASE: u32 = 1 << 31;
/// Failed logins one address may make per window before it has to wait. Tuning.
const MAX_FAILED_LOGINS: u32 = 10;
const FAILED_LOGIN_WINDOW: Duration = Duration::from_secs(600);

/// Accounts that are online now: each at most once. Released when the guard drops.
#[derive(Default)]
pub struct Online(Mutex<HashSet<PlayerId>>);

pub struct Session<'a>(&'a Online, PlayerId);

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Ok(mut online) = self.0.0.lock() {
            online.remove(&self.1);
        }
    }
}

impl Online {
    pub fn claim(&self, id: PlayerId) -> Option<Session<'_>> {
        let mut online = self.0.lock().ok()?;
        online.insert(id).then(|| Session(self, id))
    }
}

/// Failed logins per address in the current window (slows password guessing; Argon2 does the rest).
#[derive(Default)]
pub struct Failures(Mutex<HashMap<IpAddr, (u32, Instant)>>);

impl Failures {
    pub fn blocked(&self, ip: IpAddr, now: Instant) -> bool {
        let Ok(mut map) = self.0.lock() else {
            return true;
        };
        map.retain(|_, (_, since)| now.saturating_duration_since(*since) < FAILED_LOGIN_WINDOW);
        map.get(&ip).is_some_and(|(n, _)| *n >= MAX_FAILED_LOGINS)
    }

    pub fn record(&self, ip: IpAddr, now: Instant) {
        if let Ok(mut map) = self.0.lock() {
            map.entry(ip).or_insert((0, now)).0 += 1;
        }
    }
}

/// Last accepted sequence number per stream; old or repeated numbers are refused (stops replays).
#[derive(Default)]
pub struct Seq(Option<u32>);

impl Seq {
    pub fn accept(&mut self, seq: u32) -> bool {
        let ok = self.0.is_none_or(|last| seq > last);
        if ok {
            self.0 = Some(seq);
        }
        ok
    }
}

/// How long a leaving player waits for their world to save them. Tuning.
const LEAVE_SAVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Takes the player out of a world and waits until the world has saved them, so the next login
/// (here or on another server) reads their latest inventory.
async fn leave(server: &Server, world: &str, id: PlayerId) {
    let (done, saved) = tokio::sync::oneshot::channel();
    if send_to(server, world, ToWorld::Leave { id })
        && send_to(server, world, ToWorld::Flush { done })
    {
        let _ = tokio::time::timeout(LEAVE_SAVE_TIMEOUT, saved).await;
    }
}

/// Tells the client why it may not play, then ends the stream.
async fn refuse(mut send: quinn::SendStream, reason: &str) {
    let msg = ServerMsg::Kicked {
        reason: reason.into(),
    };
    let _ = write_msg(&mut send, &msg).await;
    flush(send).await;
}

/// How long a closing connection waits for its last messages (e.g. `Kicked`) to be delivered.
const FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Ends the stream and waits until the peer has it, so closing the connection does not drop it.
async fn flush(mut send: quinn::SendStream) {
    let _ = send.finish();
    let _ = tokio::time::timeout(FLUSH_TIMEOUT, send.stopped()).await;
}

pub struct Server {
    pub registry: Registry,
    pub clock: Clock,
    pub next_id: AtomicU32,
    pub slots: Slots,
    /// `MAX_PLAYERS` in production; tests lower it.
    pub max_players: u32,
    /// Development tokens that log in as staff (`GAME_ADMIN_TOKENS` / `GAME_MOD_TOKENS`); only
    /// used by `Auth::Dev`, i.e. servers without a database. Accounts carry their own role.
    pub admin_tokens: Vec<String>,
    pub mod_tokens: Vec<String>,
    /// Account names that log in as `Role::Owner` (`GAME_SUPER_ADMINS`, comma-separated).
    pub super_admins: Vec<String>,
    pub accounts: Arc<Accounts>,
    /// This server, for leases shared with other servers.
    pub identity: crate::leases::Identity,
    pub signer: crate::token::Signer,
    pub online: Online,
    /// Failed logins per address, for the HTTPS login.
    pub failures: Failures,
}

impl Server {
    /// A connection's role comes from its token alone.
    pub fn role_of(&self, token: &str) -> Role {
        if self.admin_tokens.iter().any(|t| t == token) {
            Role::Admin
        } else if self.mod_tokens.iter().any(|t| t == token) {
            Role::Moderator
        } else {
            Role::Player
        }
    }
}

pub async fn handle(server: Arc<Server>, conn: quinn::Connection) {
    let addr = conn.remote_address();
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
        return;
    };
    let kick = |reason: &str| ServerMsg::Kicked {
        reason: reason.into(),
    };

    let (auth, country) = match read_msg::<ClientMsg>(&mut recv, MAX_CLIENT_MSG).await {
        Ok(ClientMsg::Hello { protocol, .. }) if protocol != PROTOCOL_VERSION => {
            let _ = write_msg(&mut send, &kick("protocol version mismatch")).await;
            flush(send).await;
            return;
        }
        Ok(ClientMsg::Hello { auth, country, .. }) => (auth, country.filter(|c| valid_country(c))),
        _ => return,
    };
    if server.slots.online() >= server.max_players {
        return refuse(send, "server full").await;
    }
    // Identity comes from the account (or, on a development server, the connection), never from
    // anything the client claims later.
    let ip = addr.ip();
    let checked = match &auth {
        Auth::Token { token } => server
            .signer
            .verify(token)
            .ok_or("Your login expired, please log in again"),
        Auth::Login { name, password } => {
            if server.failures.blocked(ip, Instant::now()) {
                return refuse(send, "Too many failed logins, try again later").await;
            }
            let login = server.accounts.login(name, password).await;
            if login.is_err() {
                server.failures.record(ip, Instant::now());
            }
            login.map_err(|error| error.message())
        }
        Auth::Register { name, password } => {
            if server.failures.blocked(ip, Instant::now()) {
                return refuse(send, "Too many attempts, try again later").await;
            }
            // Every registration counts, like the HTTPS one, so one address cannot mass-create.
            server.failures.record(ip, Instant::now());
            server
                .accounts
                .register(name, password)
                .await
                .map_err(|error| error.message())
        }
        Auth::Dev { token, name } => {
            if !server.accounts.allows_dev() {
                return refuse(send, "This server needs an account").await;
            }
            if token.is_empty() || token.len() > MAX_TOKEN_LEN {
                return;
            }
            let id = PlayerId(DEV_ID_BASE + server.next_id.fetch_add(1, Ordering::Relaxed));
            // ponytail: dev names are taken as given (if valid) and may repeat.
            let name = if valid_name(name) {
                name.clone()
            } else {
                format!("Player{}", id.0 - DEV_ID_BASE)
            };
            Ok(crate::accounts::Account {
                id,
                name,
                role: server.role_of(token),
            })
        }
    };
    let account = match checked {
        Ok(account) => account,
        Err(reason) => return refuse(send, reason).await,
    };
    let token = if matches!(auth, Auth::Dev { .. }) {
        String::new()
    } else {
        server.signer.issue(&account)
    };
    // The super admins are named in the server's configuration only (`GAME_SUPER_ADMINS`), and only
    // real accounts qualify: a development login could pick any name.
    let owner = !matches!(auth, Auth::Dev { .. })
        && server
            .super_admins
            .iter()
            .any(|n| n.eq_ignore_ascii_case(&account.name));
    let role = if owner { Role::Owner } else { account.role };
    // Who `RefreshToken` issues for; development logins have no token.
    let refresh = (!token.is_empty()).then(|| account.clone());
    let (id, name) = (account.id, account.name);
    let Some(_session) = server.online.claim(id) else {
        return refuse(send, "That account is already online").await;
    };
    // The same account on another server counts too.
    if !server.accounts.claim_session(&server.identity, id).await {
        return refuse(send, "That account is already online").await;
    }
    // Checked above for a friendly message; taken here atomically, so a race still cannot exceed the cap.
    let Some(_slot) = server.slots.take(server.max_players) else {
        return refuse(send, "server full").await;
    };
    let welcome = ServerMsg::Welcome {
        player: id,
        server_time: server.clock.now(),
        name: name.clone(),
        token,
        role,
    };
    if write_msg(&mut send, &welcome).await.is_err() {
        return;
    }
    println!("{addr} is player {}", id.0);

    let (out, mut outbox) = mpsc::channel::<Out>(OUTBOX);
    let writer_conn = conn.clone();
    let writer = tokio::spawn(async move {
        while let Some(msg) = outbox.recv().await {
            let sent = match msg {
                Out::Datagram(msg) => {
                    let bytes = encode(&msg);
                    let fits = writer_conn
                        .max_datagram_size()
                        .is_some_and(|max| bytes.len() <= max);
                    // Too big for a datagram (a crowded world): the reliable stream still works.
                    if fits {
                        writer_conn.send_datagram(bytes.into()).is_ok()
                    } else {
                        write_msg(&mut send, &msg).await.is_ok()
                    }
                }
                Out::Reliable(msg) => write_msg(&mut send, &msg).await.is_ok(),
            };
            if !sent {
                break;
            }
        }
        flush(send).await;
    });

    let mut world: Option<String> = None;
    let now = Instant::now();
    let (mut inputs, mut acts) = (
        Bucket::new(INPUT_RATE, INPUT_BURST, now),
        Bucket::new(ACT_RATE, ACT_BURST, now),
    );
    let mut chats = Bucket::new(CHAT_RATE, CHAT_BURST, now);
    let mut lookups = Bucket::new(LOOKUP_RATE, LOOKUP_BURST, now);
    let mut voice_mutes = Bucket::new(VOICE_MUTE_RATE, VOICE_MUTE_BURST, now);
    let (mut input_seq, mut act_seq) = (Seq::default(), Seq::default());
    let mut strikes = 0;
    let reason = loop {
        let msg = match read_msg::<ClientMsg>(&mut recv, MAX_CLIENT_MSG).await {
            Ok(msg) => msg,
            Err(ReadError::Closed) => break None,
            Err(ReadError::TooLarge(_) | ReadError::Malformed) => break Some("bad message"),
        };
        let now = Instant::now();
        let to_world = match (msg, &world) {
            (ClientMsg::Join { world: wanted }, None) => {
                let world_name = wanted.to_uppercase();
                // A refused name is answered like a lookup, not punished: the deny-list can
                // refuse an innocent name, and a typo is no offence. The lookups bucket keeps
                // refused joins from hammering the database.
                let status = world_status(&server, id, &world_name).await;
                if matches!(status, WorldStatus::NotAllowed | WorldStatus::LimitReached) {
                    if lookups.take(now) {
                        let name = world_name;
                        let _ =
                            out.try_send(Out::Reliable(ServerMsg::WorldStatus { name, status }));
                    }
                    continue;
                }
                // Read after any earlier world of this session saved (see `leave`).
                let save = server.accounts.load_player(id).await.map(Box::new);
                let join = ToWorld::Join {
                    id,
                    name: name.clone(),
                    country: country.clone(),
                    role,
                    save,
                    out: out.clone(),
                };
                if !send_to(&server, &world_name, join) {
                    break Some("world unavailable");
                }
                world = Some(world_name);
                continue;
            }
            (ClientMsg::Leave, Some(_)) => {
                if let Some(name) = world.take() {
                    leave(&server, &name, id).await;
                }
                continue;
            }
            (ClientMsg::Input { seq, input }, Some(name)) => (input_seq.accept(seq)
                && inputs.take(now))
            .then(|| (name.clone(), ToWorld::Input { id, seq, input })),
            (ClientMsg::Act { seq, action }, Some(name)) => (act_seq.accept(seq) && acts.take(now))
                .then(|| (name.clone(), ToWorld::Act { id, seq, action })),
            // Empty after cleaning, or over the rate: dropped, and it counts as a strike.
            (ClientMsg::Chat { text }, Some(name)) => clean_chat(&text)
                .filter(|_| chats.take(now))
                .map(|text| (name.clone(), ToWorld::Chat { id, text })),
            // Answered here, not by the world: names live in the accounts table.
            (ClientMsg::LookupNames { ids }, _) => {
                if ids.len() > MAX_LOOKUP || !lookups.take(now) {
                    strikes += 1;
                } else {
                    let names = server.accounts.names_of(&ids).await;
                    let _ = out.try_send(Out::Reliable(ServerMsg::Names { names }));
                }
                continue;
            }
            (ClientMsg::FindPlayer { name: asked }, _) => {
                if !lookups.take(now) {
                    strikes += 1;
                } else {
                    let found = server.accounts.find_name(&asked).await;
                    let _ = out.try_send(Out::Reliable(ServerMsg::PlayerFound { asked, found }));
                }
                continue;
            }
            (ClientMsg::WorldStatus { name: asked }, _) => {
                if !lookups.take(now) {
                    strikes += 1;
                } else {
                    let name = asked.to_ascii_uppercase();
                    let status = world_status(&server, id, &name).await;
                    let _ = out.try_send(Out::Reliable(ServerMsg::WorldStatus { name, status }));
                }
                continue;
            }
            (ClientMsg::RefreshToken, _) => {
                if !lookups.take(now) {
                    strikes += 1;
                } else {
                    let token = refresh
                        .as_ref()
                        .map_or_else(String::new, |account| server.signer.issue(account));
                    let _ = out.try_send(Out::Reliable(ServerMsg::Token { token }));
                }
                continue;
            }
            (ClientMsg::ListWorlds, _) => {
                if !lookups.take(now) {
                    strikes += 1;
                } else {
                    let active = active_worlds(&server);
                    let _ = out.try_send(Out::Reliable(ServerMsg::Worlds { active }));
                }
                continue;
            }
            (ClientMsg::VoiceMute { player, muted }, Some(name)) => voice_mutes
                .take(now)
                .then(|| (name.clone(), ToWorld::VoiceMute { id, player, muted })),
            // A second Hello, a Join while in a world, anything before Join.
            _ => None,
        };
        match to_world {
            Some((name, msg)) => {
                if !send_to(&server, &name, msg) {
                    strikes += 1;
                }
            }
            None => strikes += 1,
        }
        if strikes > MAX_STRIKES {
            let _ = out.try_send(Out::Reliable(kick("too many messages")));
            break Some("too many messages");
        }
    };

    // The account stays claimed (`_session`) until its last world has saved it.
    if let Some(name) = world {
        leave(&server, &name, id).await;
    }
    server.accounts.release_session(&server.identity, id).await;
    drop(out);
    // Let the writer flush a Kicked message before the connection goes.
    let _ = writer.await;
    conn.close(0u32.into(), reason.unwrap_or("bye").as_bytes());
    println!("player {} left ({})", id.0, reason.unwrap_or("closed"));
}

/// What entering `name` (upper-case) would do for player `id`.
async fn world_status(server: &Server, id: PlayerId, name: &str) -> WorldStatus {
    if !world_name_allowed(name) {
        return WorldStatus::NotAllowed;
    }
    let loaded = server.registry.lock().ok().and_then(|worlds| {
        worlds
            .get(name)
            .map(|world| world.players.load(Ordering::Relaxed))
    });
    if let Some(players) = loaded {
        return if players >= WORLD_CAPACITY {
            WorldStatus::Full
        } else {
            WorldStatus::Open { players }
        };
    }
    if server.accounts.world_exists(name).await {
        WorldStatus::Open { players: 0 }
    } else if server.accounts.worlds_opened_last_hour(id).await >= u32::from(NEW_WORLDS_PER_HOUR) {
        WorldStatus::LimitReached
    } else {
        WorldStatus::New
    }
}

/// The busiest loaded worlds with anyone in them, most players first.
fn active_worlds(server: &Server) -> Vec<(String, u8)> {
    let Ok(worlds) = server.registry.lock() else {
        return Vec::new();
    };
    let mut active: Vec<(String, u8)> = worlds
        .iter()
        .map(|(name, world)| (name.clone(), world.players.load(Ordering::Relaxed)))
        .filter(|&(_, players)| players > 0)
        .collect();
    active.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    active.truncate(ACTIVE_WORLDS_LISTED);
    active
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bucket_allows_the_burst_then_the_rate() {
        let t0 = Instant::now();
        let mut bucket = Bucket::new(10.0, 3.0, t0);
        assert!((0..3).all(|_| bucket.take(t0)));
        assert!(!bucket.take(t0));
        assert!(bucket.take(t0 + Duration::from_millis(100)));
        assert!(!bucket.take(t0 + Duration::from_millis(100)));
        // Saving up never exceeds the burst.
        let later = t0 + Duration::from_secs(60);
        assert_eq!((0..10).filter(|_| bucket.take(later)).count(), 3);
    }

    #[test]
    fn slots_cap_and_release() {
        let slots = Slots::default();
        let a = slots.take(2).expect("first");
        let _b = slots.take(2).expect("second");
        assert!(slots.take(2).is_none(), "full");
        drop(a);
        assert_eq!(slots.online(), 1);
        assert!(slots.take(2).is_some(), "a freed slot is reusable");
    }

    #[test]
    fn seq_must_increase() {
        let mut seq = Seq::default();
        assert!(seq.accept(0));
        assert!(seq.accept(5));
        assert!(!seq.accept(5));
        assert!(!seq.accept(4));
        assert!(seq.accept(6));
    }
}
