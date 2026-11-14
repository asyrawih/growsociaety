//! Online mode (docs/knowledge/10): `GAME_SERVER=localhost cargo run -p game_client`.
//! The server decides everything; this client predicts its own movement and mirrors the world
//! from the server's `Event`s. Without `GAME_SERVER` nothing here runs and the game is offline.
//!
//! A background thread owns the QUIC connection; the game talks to it through two channels.
//! `state::apply_actions` calls `Net::act` and `Net::pump`, and `player::tick_player` calls `Net::input`,
//! so the only systems that write `Game` stay the same two as offline.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use bevy::prelude::*;
use game_protocol::{
    Auth, ChatKind, ClientMsg, DEFAULT_PORT, MAX_SERVER_MSG, PROTOCOL_VERSION, ReportReason,
    SERVER_NAME, ServerMsg, WorldStatus, decode, read_msg, write_msg,
};
use game_shared::commands::Role;
use game_shared::mirror::apply_event;
use game_shared::physics::{Input, step_with};
use game_shared::rules::{Action, Event, PlayerId, WorldState};
use game_shared::world::TileExtra;
use quinn::rustls::RootCertStore;
use quinn::rustls::pki_types::CertificateDer;
use tokio::sync::mpsc as tokio_mpsc;

use crate::state::{ActionResult, Game, GameEvent};

const SERVER_ENV: &str = "GAME_SERVER";
const WORLD_ENV: &str = "GAME_WORLD";
const TOKEN_ENV: &str = "GAME_TOKEN";
/// Wanted display name. Unset or invalid: the server names you `Player<id>`.
const NAME_ENV: &str = "GAME_NAME";
/// An account to log in with (e.g. `demo1` / `demo1`). Without them a development login is tried,
/// which only servers without a database accept.
pub const USER_ENV: &str = "GAME_USER";
pub const PASSWORD_ENV: &str = "GAME_PASSWORD";
const CERT_ENV: &str = "GAME_SERVER_CERT";
/// A login token from the HTTP login API; set by the lobby when it restarts the game.
pub const AUTH_TOKEN_ENV: &str = "GAME_AUTH_TOKEN";
/// Written by `world_server` at start; relative to the directory both are run from.
const CERT_FILE: &str = "world_server_cert.der";
/// Release builds play online here unless `GAME_SERVER` says otherwise; debug builds stay offline.
pub const PROD_SERVER: &str = "76.13.196.13:7777";
/// Production's public certificate (never the key), pinned so a packaged client needs no files.
/// The client always verifies it; it is issued for `SERVER_NAME`, so it also holds behind a domain.
const PROD_CERT: &[u8] = include_bytes!("../certs/prod.der");

/// The server to join: `GAME_SERVER`, else production in release builds, else none (offline).
pub fn server_address() -> Option<String> {
    std::env::var(SERVER_ENV)
        .ok()
        .or_else(|| (!cfg!(debug_assertions)).then(|| PROD_SERVER.to_owned()))
}
const DEFAULT_WORLD: &str = "START";
const DEFAULT_TOKEN: &str = "dev";
/// The server's refusal while the account's previous session is still open (world_server conn.rs).
const ALREADY_ONLINE: &str = "That account is already online";
/// How long, and how often, a refused login is retried while that session closes.
const ONLINE_RETRY: Duration = Duration::from_secs(5);
const ONLINE_RETRY_EVERY: Duration = Duration::from_millis(250);
/// `Net::close` waits at most this long for the server to hear the close.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// How long startup waits for the server's snapshot.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Inputs kept for replay; ~4 s at 60 Hz, far more than any round trip.
const INPUT_HISTORY: usize = 256;

/// The connection, as the game sees it.
#[derive(Resource)]
pub struct Net {
    to_server: tokio_mpsc::UnboundedSender<ClientMsg>,
    from_server: Mutex<mpsc::Receiver<ServerMsg>>,
    /// Server clock minus local clock (`Time::elapsed_secs_f64`).
    /// ponytail: measured at Welcome against a local clock that starts a moment later, at the first
    /// frame; the error (startup time) only shifts tree growth. Ping-based sync if it ever matters.
    clock_offset: f64,
    next_act: u32,
    pending: HashMap<u32, Action>,
    next_input: u32,
    inputs: VecDeque<(u32, Input)>,
    last_ack: u32,
    connected: bool,
    /// Chat lines received, until `chat.rs` takes them.
    chat: Vec<(PlayerId, String, ChatKind)>,
    /// Display names of everyone in the world, as the server sent them.
    names: HashMap<PlayerId, String>,
    /// Country codes, for flags.
    countries: HashMap<PlayerId, String>,
    /// Our own role, as the server said at login (for which commands to list).
    role: Role,
    /// Names of players not in the world (offline claim owners, access lists), from `lookup_names`.
    known_names: HashMap<PlayerId, String>,
    /// Ids already asked about, so each is looked up once.
    asked: std::collections::HashSet<PlayerId>,
    /// Answers to `find_player` (name asked, account found), until taken.
    found: Vec<(String, Option<(PlayerId, String)>)>,
    /// The newest `WorldStatus` and `Worlds` answers, until the world menu takes them.
    world_status: Option<(String, WorldStatus)>,
    worlds: Option<Vec<(String, u8)>>,
    /// Set when `connect` had to fall back; `NetPlugin` turns it into the resource.
    join_refused: Option<JoinRefused>,
    /// Other players' swings (player, x, y, place) until `player.rs` takes them.
    swings: Vec<(PlayerId, u32, u32, bool)>,
    /// The answer to `refresh_token`, until taken.
    token: Option<String>,
    /// Everyone this player has muted with `/ignore`, from the server (kept across logins).
    ignoring: std::collections::HashSet<PlayerId>,
    /// Other players' roles, from the join snapshot and `PlayerJoined`.
    roles: HashMap<PlayerId, Role>,
    /// The newest voice token (url, token) and world-owner mutes, until `voice.rs` takes them.
    voice_token: Option<(String, String)>,
    voice_muted: Vec<(PlayerId, bool)>,
    /// The connection's thread; `close` waits for it.
    session: Option<std::thread::JoinHandle<()>>,
}

/// What the background thread hands over once joined: id, server clock, world, (names,
/// countries, roles).
type Joined = (
    PlayerId,
    f64,
    WorldState,
    (
        HashMap<PlayerId, String>,
        HashMap<PlayerId, String>,
        HashMap<PlayerId, Role>,
    ),
    Role,
    Option<JoinRefused>,
);

/// The world asked for (`GAME_WORLD`) refused the join, so the game joined a fallback world
/// instead (`GAME_PREV_WORLD`, else START): never a "no world" state. Inserted once at startup;
/// the world menu shows why and removes it (docs/knowledge/ux/world-select.md).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct JoinRefused {
    /// The refused world, upper-case.
    pub name: String,
    pub reason: Refusal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    Status(WorldStatus),
    Kicked(String),
}

/// The world a re-exec came from, for the fallback join.
const PREV_WORLD_ENV: &str = "GAME_PREV_WORLD";

/// The local player's country code: `GAME_COUNTRY`, else the region of the system locale
/// (`LANG=id_ID.UTF-8` gives `id`). `None` if neither gives a valid code.
pub fn local_country() -> Option<String> {
    let from_env = std::env::var("GAME_COUNTRY").ok().map(|c| c.to_lowercase());
    let from_locale = || {
        let lang = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .ok()?;
        let region = lang.split(['.', '@']).next()?.split('_').nth(1)?;
        Some(region.to_lowercase())
    };
    from_env
        .or_else(from_locale)
        .filter(|code| game_protocol::valid_country(code))
}

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        let Some(server) = server_address() else {
            return;
        };
        let world = std::env::var(WORLD_ENV).unwrap_or_else(|_| DEFAULT_WORLD.into());
        // Explicitly asked to play online: failing to connect is fatal, not a silent offline game.
        // A refused world already fell back (`JoinRefused`); this is the server unreachable or the
        // fallback refused too. Logged (a started game has no terminal) and a clean exit, not a panic.
        let (net, state, me) = match connect(&server, &world) {
            Ok(joined) => joined,
            Err(err) => {
                error!("cannot join {world} on {server}: {err}");
                std::process::exit(1);
            }
        };
        info!("online: player {} in {world} on {server}", me.0);
        let mut net = net;
        if let Some(refused) = net.join_refused.take() {
            app.insert_resource(refused);
        }
        app.insert_resource(Game(state, me)).insert_resource(net);
    }
}

/// Server times in a message, moved onto the local clock.
fn to_local_event(event: &mut Event, offset: f64) {
    if let Event::TreePlanted { tree, .. } | Event::TreeSpliced { tree, .. } = event {
        tree.planted_at -= offset;
    }
}

fn to_local_state(state: &mut WorldState, offset: f64) {
    for extra in state.world.extras.values_mut() {
        if let TileExtra::Tree(tree) = extra {
            tree.planted_at -= offset;
        }
    }
    for damage in state.damage.values_mut() {
        damage.last_hit -= offset;
    }
    for player in state.players.values_mut() {
        player.last_punch -= offset;
    }
}

/// The server's address and a client config trusting its certificate. Also used by the lobby.
pub fn quic_target(server: &str) -> Result<(SocketAddr, quinn::ClientConfig), String> {
    let with_port = if server.contains(':') {
        server.to_owned()
    } else {
        format!("{server}:{DEFAULT_PORT}")
    };
    let addr = with_port
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("no address")?;
    // `GAME_SERVER_CERT` first; production gets its embedded certificate; anything else the file
    // a local `world_server` wrote.
    let cert = match std::env::var(CERT_ENV) {
        Ok(path) => std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?,
        Err(_) if server == PROD_SERVER => PROD_CERT.to_vec(),
        Err(_) => std::fs::read(CERT_FILE).map_err(|e| format!("{CERT_FILE}: {e}"))?,
    };
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(cert))
        .map_err(|e| e.to_string())?;
    let config =
        quinn::ClientConfig::with_root_certificates(Arc::new(roots)).map_err(|e| e.to_string())?;
    Ok((addr, config))
}

/// Blocks until the server has sent the world, then leaves the connection to a background thread.
/// What a joined world hands over: its state and (names, countries, roles).
type JoinedWorld = (
    WorldState,
    (
        HashMap<PlayerId, String>,
        HashMap<PlayerId, String>,
        HashMap<PlayerId, Role>,
    ),
);

/// Asks to join `world`. `Ok(Err(refusal))` when the server says no (a world status or a kick);
/// `Err` when the connection itself fails.
async fn join(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    world: &str,
) -> Result<Result<JoinedWorld, Refusal>, String> {
    let world = world.to_owned();
    write_msg(send, &ClientMsg::Join { world })
        .await
        .map_err(|e| e.to_string())?;
    match read_msg::<ServerMsg>(recv, MAX_SERVER_MSG).await {
        Ok(ServerMsg::Joined { snapshot }) => {
            let names: HashMap<_, _> = snapshot.names.iter().cloned().collect();
            let countries = snapshot.countries.iter().cloned().collect();
            let roles = snapshot.roles.iter().copied().collect();
            Ok(Ok((snapshot.into_state(), (names, countries, roles))))
        }
        Ok(ServerMsg::WorldStatus { status, .. }) => Ok(Err(Refusal::Status(status))),
        Ok(ServerMsg::Kicked { reason }) => Ok(Err(Refusal::Kicked(reason))),
        other => Err(format!("unexpected answer to Join: {other:?}")),
    }
}

fn connect(server: &str, world: &str) -> Result<(Net, WorldState, PlayerId), String> {
    let (addr, config) = quic_target(server)?;
    let auth = if let Ok(token) = std::env::var(AUTH_TOKEN_ENV) {
        Auth::Token { token }
    } else if let (Ok(name), Ok(password)) = (std::env::var(USER_ENV), std::env::var(PASSWORD_ENV))
    {
        Auth::Login { name, password }
    } else {
        Auth::Dev {
            token: std::env::var(TOKEN_ENV).unwrap_or_else(|_| DEFAULT_TOKEN.into()),
            name: std::env::var(NAME_ENV).unwrap_or_default(),
        }
    };
    let world = world.to_owned();

    let (to_server, mut outgoing) = tokio_mpsc::unbounded_channel::<ClientMsg>();
    let (incoming_tx, from_server) = mpsc::channel::<ServerMsg>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<Joined, String>>();

    let session = std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        runtime.block_on(async move {
            let local = if addr.is_ipv6() {
                "[::]:0"
            } else {
                "0.0.0.0:0"
            };
            let session = async {
                let mut endpoint =
                    quinn::Endpoint::client(local.parse().map_err(|_| "bind address")?)
                        .map_err(|e| e.to_string())?;
                endpoint.set_default_client_config(config);
                // A world switch re-starts the game: the old process's session may still be
                // closing on the server for a moment, so "already online" is retried briefly.
                let retry_until = tokio::time::Instant::now() + ONLINE_RETRY;
                let (conn, mut send, mut recv, me, server_time, role) = loop {
                    let conn = endpoint
                        .connect(addr, SERVER_NAME)
                        .map_err(|e| e.to_string())?
                        .await
                        .map_err(|e| e.to_string())?;
                    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
                    let hello = ClientMsg::Hello {
                        protocol: PROTOCOL_VERSION,
                        auth: auth.clone(),
                        country: local_country(),
                    };
                    write_msg(&mut send, &hello)
                        .await
                        .map_err(|e| e.to_string())?;
                    match read_msg::<ServerMsg>(&mut recv, MAX_SERVER_MSG).await {
                        Ok(ServerMsg::Welcome {
                            player,
                            server_time,
                            role,
                            ..
                        }) => break (conn, send, recv, player, server_time, role),
                        Ok(ServerMsg::Kicked { reason })
                            if reason == ALREADY_ONLINE
                                && tokio::time::Instant::now() < retry_until =>
                        {
                            info!("{reason}: the previous session is still closing, retrying");
                            tokio::time::sleep(ONLINE_RETRY_EVERY).await;
                        }
                        Ok(ServerMsg::Kicked { reason }) => return Err(reason),
                        other => return Err(format!("unexpected answer to Hello: {other:?}")),
                    }
                };
                let (joined, refused) = match join(&mut send, &mut recv, &world).await? {
                    Ok(joined) => (joined, None),
                    // Refused: join where the player came from, else START, and say why.
                    Err(reason) => {
                        let fallback = std::env::var(PREV_WORLD_ENV)
                            .ok()
                            .map(|name| name.to_ascii_uppercase())
                            .filter(|name| *name != world.to_ascii_uppercase())
                            .unwrap_or_else(|| DEFAULT_WORLD.to_owned());
                        warn!("{world} refused the join ({reason:?}), joining {fallback}");
                        // The server may count us as in the refused world; leaving is harmless if not.
                        write_msg(&mut send, &ClientMsg::Leave)
                            .await
                            .map_err(|e| e.to_string())?;
                        let joined =
                            join(&mut send, &mut recv, &fallback)
                                .await?
                                .map_err(|again| {
                                    format!("{fallback} refused the join too: {again:?}")
                                })?;
                        let name = world.to_ascii_uppercase();
                        (joined, Some(JoinRefused { name, reason }))
                    }
                };
                let (state, names) = joined;
                Ok((
                    endpoint,
                    conn,
                    send,
                    recv,
                    (me, server_time, state, names, role, refused),
                ))
            };
            let (endpoint, conn, mut send, mut recv, joined) = match session.await {
                Ok(parts) => parts,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(joined));

            let stream_tx = incoming_tx.clone();
            let reader = async move {
                while let Ok(msg) = read_msg::<ServerMsg>(&mut recv, MAX_SERVER_MSG).await {
                    if stream_tx.send(msg).is_err() {
                        break;
                    }
                }
            };
            let datagrams = async {
                while let Ok(bytes) = conn.read_datagram().await {
                    if let Ok(msg) = decode::<ServerMsg>(&bytes)
                        && incoming_tx.send(msg).is_err()
                    {
                        break;
                    }
                }
            };
            let writer = async move {
                while let Some(msg) = outgoing.recv().await {
                    if write_msg(&mut send, &msg).await.is_err() {
                        break;
                    }
                }
            };
            // Whichever side ends first ends the session; the game sees the channel close.
            tokio::select! {
                _ = reader => {}
                _ = datagrams => {}
                _ = writer => {}
            }
            conn.close(0u32.into(), b"bye");
            // Deliver the close, so the server frees the account now (`Net::close`), not after
            // its idle timeout.
            let _ = tokio::time::timeout(CLOSE_WAIT, endpoint.wait_idle()).await;
        });
    });

    let (me, server_time, mut state, (names, countries, roles), role, join_refused) = ready_rx
        .recv_timeout(JOIN_TIMEOUT)
        .map_err(|_| "timed out waiting for the server".to_owned())??;
    // The local clock is ~0 at startup.
    let clock_offset = server_time;
    to_local_state(&mut state, clock_offset);
    let net = Net {
        to_server,
        from_server: Mutex::new(from_server),
        clock_offset,
        next_act: 0,
        pending: HashMap::new(),
        next_input: 0,
        inputs: VecDeque::new(),
        last_ack: 0,
        connected: true,
        chat: Vec::new(),
        names,
        countries,
        role,
        known_names: HashMap::new(),
        asked: Default::default(),
        found: Vec::new(),
        world_status: None,
        worlds: None,
        join_refused,
        swings: Vec::new(),
        token: None,
        ignoring: Default::default(),
        roles,
        voice_token: None,
        voice_muted: Vec::new(),
        session: Some(session),
    };
    Ok((net, state, me))
}

impl Net {
    /// Asks the server; the answer comes back through `pump` as an `ActionResult`.
    pub fn act(&mut self, action: Action) {
        self.next_act += 1;
        let seq = self.next_act;
        self.pending.insert(seq, action);
        let _ = self.to_server.send(ClientMsg::Act { seq, action });
    }

    /// Says something in the world; the server echoes it back, cleaned, to everyone.
    pub fn chat(&mut self, text: String) {
        let _ = self.to_server.send(ClientMsg::Chat { text });
    }

    /// A player's display name, if the server has told us.
    pub fn country(&self, id: PlayerId) -> Option<&str> {
        self.countries.get(&id).map(String::as_str)
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// Another player's role in this world, from the server (`None` if unknown). Our own is
    /// `role()`.
    pub fn role_of(&self, player: PlayerId) -> Option<Role> {
        self.roles.get(&player).copied()
    }

    /// Everyone in the world, by display name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.values().map(String::as_str)
    }

    /// A player's display name: someone in the world, else one learned by `lookup_names`.
    pub fn name(&self, id: PlayerId) -> Option<&str> {
        self.names
            .get(&id)
            .or_else(|| self.known_names.get(&id))
            .map(String::as_str)
    }

    /// Asks the server for the names of `ids` this client cannot name yet (each id once). The
    /// answer arrives in a later frame; `name` returns it from then on.
    /// Asks the server for the account called `name` (the claim panel's Add field). The answer
    /// comes back through `take_found`.
    #[allow(dead_code)] // the claim panel (hud.rs, growsociaety-1a) calls it; not wired yet
    pub fn find_player(&mut self, name: &str) {
        let _ = self.to_server.send(ClientMsg::FindPlayer {
            name: name.to_owned(),
        });
    }

    /// `find_player` answers since the last call: (name asked, Some((id, exact name)) or None).
    #[allow(dead_code)] // as above
    pub fn take_found(&mut self) -> Vec<(String, Option<(PlayerId, String)>)> {
        std::mem::take(&mut self.found)
    }

    /// Asks what entering world `name` would do (docs/knowledge/ux/world-select.md). Debounce
    /// it: the server rate-limits lookups and drops what is over the limit.
    pub fn world_status(&mut self, name: String) {
        let _ = self.to_server.send(ClientMsg::WorldStatus { name });
    }

    /// The newest `world_status` answer, once: (name upper-case, status).
    pub fn take_world_status(&mut self) -> Option<(String, WorldStatus)> {
        self.world_status.take()
    }

    /// Ends the session and waits (at most `CLOSE_WAIT`) until the server has heard it, so it frees
    /// the account at once. Call it before the process exits for a world switch.
    pub fn close(&mut self) {
        // The writer ends once its last sender is gone, which ends the session.
        self.to_server = tokio_mpsc::unbounded_channel().0;
        if let Some(session) = self.session.take() {
            let _ = session.join();
        }
    }

    /// Asks the server for a fresh login token; `take_token` has it a moment later. Call it right
    /// before re-starting the game for a world switch and pass it on as `GAME_AUTH_TOKEN`.
    pub fn refresh_token(&mut self) {
        let _ = self.to_server.send(ClientMsg::RefreshToken);
    }

    /// The fresh token from `refresh_token`, once (empty on development logins).
    pub fn take_token(&mut self) -> Option<String> {
        self.token.take()
    }

    /// Reports `player` to staff (the avatar menu). The answer is a System chat line.
    pub fn report(&mut self, player: PlayerId, reason: ReportReason, note: Option<String>) {
        let _ = self.to_server.send(ClientMsg::Report {
            player,
            reason,
            note,
        });
    }

    /// Whether this player has muted `player` with `/ignore` (kept across logins): the avatar
    /// menu shows Unmute for them, and their voice stays muted.
    pub fn is_ignoring(&self, player: PlayerId) -> bool {
        self.ignoring.contains(&player)
    }

    /// Everyone this player has muted, e.g. to mute their voices on joining.
    #[cfg_attr(not(feature = "voice"), allow(dead_code))] // voice re-applies the mutes
    pub fn ignoring(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.ignoring.iter().copied()
    }

    /// Other players' swings since the last call: (player, x, y, place).
    pub fn take_swings(&mut self) -> Vec<(PlayerId, u32, u32, bool)> {
        std::mem::take(&mut self.swings)
    }

    /// Asks for the busiest loaded worlds.
    pub fn list_worlds(&mut self) {
        let _ = self.to_server.send(ClientMsg::ListWorlds);
    }

    /// The newest `list_worlds` answer, once: (name, players), most players first.
    pub fn take_worlds(&mut self) -> Option<Vec<(String, u8)>> {
        self.worlds.take()
    }

    #[allow(dead_code)] // the claim panel (hud.rs, growsociaety-1a) calls it; not wired yet
    pub fn lookup_names(&mut self, ids: impl IntoIterator<Item = PlayerId>) {
        let new: Vec<PlayerId> = ids
            .into_iter()
            .filter(|id| self.name(*id).is_none() && self.asked.insert(*id))
            .take(game_protocol::MAX_LOOKUP)
            .collect();
        if !new.is_empty() {
            let _ = self.to_server.send(ClientMsg::LookupNames { ids: new });
        }
    }

    /// Chat lines received since the last call.
    pub fn take_chat(&mut self) -> Vec<(PlayerId, String, ChatKind)> {
        std::mem::take(&mut self.chat)
    }

    /// The voice token received since the last call: (LiveKit url, token).
    #[cfg_attr(not(feature = "voice"), allow(dead_code))]
    pub fn take_voice_token(&mut self) -> Option<(String, String)> {
        self.voice_token.take()
    }

    /// World-owner voice mutes received since the last call.
    #[cfg_attr(not(feature = "voice"), allow(dead_code))]
    pub fn take_voice_muted(&mut self) -> Vec<(PlayerId, bool)> {
        std::mem::take(&mut self.voice_muted)
    }

    /// Asks the server to mute or unmute `player`'s voice (world owner only; the server checks).
    #[cfg_attr(not(feature = "voice"), allow(dead_code))]
    pub fn voice_mute(&mut self, player: PlayerId, muted: bool) {
        let _ = self.to_server.send(ClientMsg::VoiceMute { player, muted });
    }

    /// Whether the connection to the server is still up.
    #[cfg_attr(not(feature = "voice"), allow(dead_code))]
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Sends one tick of input and remembers it for replay.
    pub fn input(&mut self, input: Input) {
        self.next_input += 1;
        let seq = self.next_input;
        if self.inputs.len() == INPUT_HISTORY {
            self.inputs.pop_front();
        }
        self.inputs.push_back((seq, input));
        let _ = self.to_server.send(ClientMsg::Input { seq, input });
    }

    /// Applies everything the server sent since the last frame.
    /// Applies what the server sent. `true` when it replaced the whole world (`/resetworld`).
    pub fn pump(
        &mut self,
        game: &mut Game,
        now: f64,
        events: &mut MessageWriter<GameEvent>,
        results: &mut MessageWriter<ActionResult>,
    ) -> bool {
        let mut replaced = false;
        let received: Vec<ServerMsg> = match self.from_server.lock() {
            Ok(rx) => std::iter::from_fn(|| rx.try_recv().ok()).collect(),
            Err(_) => Vec::new(),
        };
        if received.is_empty() && self.connected && self.to_server.is_closed() {
            self.connected = false;
            error!("disconnected from the server");
        }
        let offset = self.clock_offset;
        let mut record = |game: &mut Game, list: &mut Vec<Event>| {
            for event in list.iter_mut() {
                to_local_event(event, offset);
                apply_event(&mut game.0, event, now);
                events.write(GameEvent(event.clone()));
            }
        };
        for msg in received {
            match msg {
                ServerMsg::Events { events: mut list } => record(game, &mut list),
                ServerMsg::Acted { seq, mut result } => {
                    let Some(action) = self.pending.remove(&seq) else {
                        continue;
                    };
                    if let Ok(list) = &mut result {
                        record(game, list);
                    }
                    crate::state::log_result(action, &result);
                    results.write(ActionResult { action, result });
                }
                ServerMsg::PlayerJoined {
                    id,
                    body,
                    equipped,
                    level,
                    name,
                    country,
                    role,
                } => {
                    self.roles.insert(id, role);
                    self.names.insert(id, name);
                    if let Some(code) = country {
                        self.countries.insert(id, code);
                    }
                    game.0.join(id);
                    if let Some(player) = game.0.players.get_mut(&id) {
                        player.body = body;
                        player.equipped = equipped;
                        player.level = level;
                    }
                }
                ServerMsg::PlayerLeft { id } => {
                    self.names.remove(&id);
                    self.countries.remove(&id);
                    game.0.players.remove(&id);
                }
                ServerMsg::Bodies {
                    ack_seq,
                    you,
                    others,
                } => {
                    // Datagrams can arrive out of order: only newer news counts.
                    if ack_seq < self.last_ack {
                        continue;
                    }
                    self.last_ack = ack_seq;
                    for (id, body) in others {
                        if let Some(player) = game.0.players.get_mut(&id) {
                            player.body = body;
                        }
                    }
                    self.reconcile(game, ack_seq, you);
                }
                ServerMsg::Kicked { reason } => {
                    self.connected = false;
                    error!("kicked by the server: {reason}");
                }
                ServerMsg::Chat { from, text, kind } => self.chat.push((from, text, kind)),
                ServerMsg::Names { names } => self.known_names.extend(names),
                ServerMsg::PlayerFound { asked, found } => {
                    if let Some((id, name)) = &found {
                        self.known_names.insert(*id, name.clone());
                    }
                    self.found.push((asked, found));
                }
                ServerMsg::WorldStatus { name, status } => self.world_status = Some((name, status)),
                ServerMsg::Worlds { active } => self.worlds = Some(active),
                ServerMsg::Token { token } => self.token = Some(token),
                ServerMsg::Ignoring { ids } => self.ignoring = ids.into_iter().collect(),
                ServerMsg::Swing {
                    player,
                    x,
                    y,
                    place,
                } => self.swings.push((player, x, y, place)),
                ServerMsg::VoiceToken { url, token, .. } => self.voice_token = Some((url, token)),
                ServerMsg::VoiceMuted { player, muted } => self.voice_muted.push((player, muted)),
                // Joined again in the same world: it was reset. Swap the world in whole; the old
                // drops leave as `DropTaken` by nobody (no pickup sound).
                ServerMsg::Joined { snapshot } => {
                    self.names.extend(snapshot.names.iter().cloned());
                    self.roles.extend(snapshot.roles.iter().copied());
                    self.countries.extend(snapshot.countries.iter().cloned());
                    let mut gone: Vec<Event> = (game.0.world.drops.iter())
                        .map(|drop| Event::DropTaken {
                            drop_id: drop.id,
                            by: PlayerId(u32::MAX),
                            count: 0,
                        })
                        .collect();
                    game.0 = snapshot.into_state();
                    record(game, &mut gone);
                    replaced = true;
                }
                ServerMsg::Welcome { .. } => {}
            }
        }
        replaced
    }

    /// Snap to the server's body and replay the inputs it has not stepped yet. Both sides run the
    /// same deterministic `step`, so this is invisible unless the world changed under us.
    fn reconcile(&mut self, game: &mut Game, ack_seq: u32, mut body: game_shared::physics::Body) {
        while self.inputs.front().is_some_and(|&(seq, _)| seq <= ack_seq) {
            self.inputs.pop_front();
        }
        let me = game.1;
        let mods = game
            .0
            .players
            .get(&me)
            .map(|p| game_shared::mods::worn_mods(&p.equipped))
            .unwrap_or_default();
        for &(_, input) in &self.inputs {
            step_with(&mut body, input, &game.0.world, mods);
        }
        if let Some(player) = game.0.players.get_mut(&me) {
            player.body = body;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::physics::step;
    use game_shared::worldgen::generate;

    fn offline_net() -> (
        Net,
        mpsc::Sender<ServerMsg>,
        tokio_mpsc::UnboundedReceiver<ClientMsg>,
    ) {
        let (to_server, sent) = tokio_mpsc::unbounded_channel();
        let (tx, rx) = mpsc::channel();
        let net = Net {
            to_server,
            from_server: Mutex::new(rx),
            clock_offset: 0.0,
            next_act: 0,
            pending: HashMap::new(),
            next_input: 0,
            inputs: VecDeque::new(),
            last_ack: 0,
            connected: true,
            chat: Vec::new(),
            names: HashMap::new(),
            countries: HashMap::new(),
            role: Role::Player,
            known_names: HashMap::new(),
            asked: Default::default(),
            found: Vec::new(),
            world_status: None,
            worlds: None,
            join_refused: None,
            swings: Vec::new(),
            token: None,
            ignoring: Default::default(),
            roles: HashMap::new(),
            voice_token: None,
            voice_muted: Vec::new(),
            session: None,
        };
        (net, tx, sent)
    }

    #[test]
    fn reconcile_replays_unacknowledged_inputs_from_the_server_body() {
        let me = PlayerId(3);
        let mut state = WorldState::new(generate("PREDICT"), 0);
        state.join(me);
        let mut game = Game(state, me);
        let (mut net, _tx, mut sent) = offline_net();
        let right = Input {
            move_x: 1,
            ..Input::default()
        };

        // Predict 10 ticks locally, exactly like `tick_player`.
        let mut server_body = game.me().expect("joined").body;
        for _ in 0..10 {
            net.input(right);
            let body = &mut game.0.players.get_mut(&me).expect("joined").body;
            step(body, right, &game.0.world);
        }
        let predicted = game.me().expect("joined").body;
        assert_eq!(std::iter::from_fn(|| sent.try_recv().ok()).count(), 10);

        // The server has stepped the first 4; replaying the other 6 lands on the prediction.
        for _ in 0..4 {
            step(&mut server_body, right, &game.0.world);
        }
        net.reconcile(&mut game, 4, server_body);
        assert_eq!(game.me().expect("joined").body, predicted);
        assert_eq!(net.inputs.len(), 6);
    }
}
