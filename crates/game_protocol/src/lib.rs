//! The wire between `world_server` and `game_client` (docs/knowledge/10). Only the envelope lives here;
//! every payload type is `game_shared`'s. postcard over QUIC: one reliable bidirectional stream per
//! connection carries length-prefixed messages both ways, datagrams carry `Bodies`.

use std::collections::HashMap;

use game_shared::items::{ItemId, SLOT_COUNT};
use game_shared::physics::{Body, Input};
use game_shared::rules::{Action, Damage, Event, Player, PlayerId, Reject, WorldState};
use game_shared::world::World;
use serde::{Deserialize, Serialize};

/// Bumped on any incompatible change; a mismatch is refused at `Hello`.
pub const PROTOCOL_VERSION: u16 = 11;
pub const DEFAULT_PORT: u16 = 7777;
/// The name the dev certificate is issued for and the client connects to.
pub const SERVER_NAME: &str = "localhost";

/// Largest client message the server reads. Checked before allocating.
pub const MAX_CLIENT_MSG: usize = 1024;
/// Largest server message the client reads: a full snapshot fits with room to spare.
pub const MAX_SERVER_MSG: usize = 1 << 20;
/// Hard limits on client strings.
pub const MAX_TOKEN_LEN: usize = 128;
/// A chat line, in bytes after cleaning (docs/knowledge/13: a command line is at most 256 bytes).
pub const MAX_CHAT_LEN: usize = 256;
/// Display names: 1 to 16 ASCII letters, digits or underscores.
pub const MAX_NAME_LEN: usize = 16;

pub fn valid_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A country (flag) code: ISO 3166-1 alpha-2 like `id`, or a region file name like `england`.
/// Lower-case ASCII letters, at most 16. Whether a flag exists for it is the client's business.
pub fn valid_country(code: &str) -> bool {
    (2..=16).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase())
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum ClientMsg {
    Hello {
        protocol: u16,
        auth: Auth,
        /// Shown as a flag next to the name; dropped unless `valid_country`.
        country: Option<String>,
    },
    Join {
        world: String,
    },
    Leave,
    /// One physics tick of input. `seq` increases by one per tick.
    Input {
        seq: u32,
        input: Input,
    },
    /// `seq` increases per action; the answer is `Acted` with the same `seq`.
    Act {
        seq: u32,
        action: Action,
    },
    /// Plain chat, or a `/command` once the server parses them (docs/knowledge/13).
    Chat {
        text: String,
    },
    /// World claim owner only (the server checks): mute or unmute `player`'s voice for everyone.
    VoiceMute {
        player: PlayerId,
        muted: bool,
    },
    /// Display names for ids the client has seen but nobody here named (offline claim owners,
    /// access lists). At most `MAX_LOOKUP` ids; the answer is `Names`.
    LookupNames {
        ids: Vec<PlayerId>,
    },
    /// The account called `name` (case-insensitive), for the claim panel's "Add" field
    /// (docs/knowledge/ux/claims.md). The answer is `PlayerFound`.
    FindPlayer {
        name: String,
    },
    /// What entering world `name` would do, for the world menu's status line. Answer: `WorldStatus`.
    WorldStatus {
        name: String,
    },
    /// The busiest loaded worlds. Answer: `Worlds`.
    ListWorlds,
    /// A fresh login token for this account, asked right before a world switch re-starts the
    /// game (tokens are short-lived). Answer: `Token`.
    RefreshToken,
}

/// Players allowed in one world at a time.
pub const WORLD_CAPACITY: u8 = 64;
/// Worlds an account may open per hour (`WorldStatus::LimitReached`).
pub const NEW_WORLDS_PER_HOUR: u8 = 10;
/// Entries in `ServerMsg::Worlds`.
pub const ACTIVE_WORLDS_LISTED: usize = 10;

/// What entering a world would do (docs/knowledge/ux/world-select.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WorldStatus {
    /// It exists; `players` are in it now.
    Open { players: u8 },
    /// Nobody has built there: entering creates it.
    New,
    /// `WORLD_CAPACITY` players are in it.
    Full,
    /// An invalid or denied name.
    NotAllowed,
    /// New, and this account opened `NEW_WORLDS_PER_HOUR` worlds in the last hour.
    LimitReached,
}

/// Ids per `LookupNames`.
pub const MAX_LOOKUP: usize = 64;

/// Who is connecting (docs/knowledge/10, "Authentication").
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum Auth {
    /// A login token from the HTTPS API (`/api/login`, `/api/register`), for when the web login
    /// is reachable.
    Token { token: String },
    /// Name and password on the game port itself. QUIC is TLS with a pinned certificate, so the
    /// password is encrypted in transit; used while the HTTPS login is not deployed.
    Login { name: String, password: String },
    /// A new account (same rules as the HTTPS `/api/register`), then logged in like `Login`.
    Register { name: String, password: String },
    /// Development servers without a database only: any token, any valid name, no account.
    Dev { token: String, name: String },
}

/// Password length in bytes. Reference.
pub const MIN_PASSWORD_LEN: usize = 8;
pub const MAX_PASSWORD_LEN: usize = 128;

pub fn valid_password(password: &str) -> bool {
    (MIN_PASSWORD_LEN..=MAX_PASSWORD_LEN).contains(&password.len())
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatKind {
    /// Said by a player to everyone in the world.
    World,
    /// From the server itself (`from` is the receiver).
    System,
    /// A `/msg` or `/r` from `from` to the receiver only.
    Private,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub enum ServerMsg {
    /// `server_time` is the clock every `now`, `planted_at` and `last_hit` in later messages is on.
    Welcome {
        player: PlayerId,
        server_time: f64,
        /// The display name the server settled on.
        name: String,
        /// A short-lived login token for this account (empty on development logins), so a
        /// launcher can hand the game a token instead of the password.
        token: String,
        /// The player's own role, so the client lists only the commands it may use. The server
        /// still checks every command.
        role: game_shared::commands::Role,
    },
    Joined {
        snapshot: Box<WorldSnapshot>,
    },
    /// Changes caused by someone else (or by the server, e.g. a pickup after movement).
    Events {
        events: Vec<Event>,
    },
    /// The answer to the receiver's own `Act`. Its events are not repeated in `Events`.
    Acted {
        seq: u32,
        result: Result<Vec<Event>, Reject>,
    },
    PlayerJoined {
        id: PlayerId,
        body: Body,
        equipped: Equipped,
        /// Public: it colours the name (docs/knowledge/14).
        level: u16,
        name: String,
        country: Option<String>,
    },
    PlayerLeft {
        id: PlayerId,
    },
    /// Authoritative bodies. `ack_seq` is the last `Input` the server stepped for the receiver.
    Bodies {
        ack_seq: u32,
        you: Body,
        others: Vec<(PlayerId, Body)>,
    },
    Kicked {
        reason: String,
    },
    /// The answer to `LookupNames`: the ids that are accounts, with their names.
    Names {
        names: Vec<(PlayerId, String)>,
    },
    /// The answer to `FindPlayer`: the name as asked, and the account's id and exact name, if any.
    PlayerFound {
        asked: String,
        found: Option<(PlayerId, String)>,
    },
    /// The answer to `WorldStatus` (docs/knowledge/ux/world-select.md).
    WorldStatus {
        name: String,
        status: WorldStatus,
    },
    /// Another player in this world punched (or placed at) tile `(x, y)`, accepted or not: their
    /// arm reaches out (docs/knowledge/ux/vfx-farming-punch.md (a)). A datagram, never sent back
    /// to the player who swung.
    Swing {
        player: PlayerId,
        x: u32,
        y: u32,
        place: bool,
    },
    /// The answer to `RefreshToken`; empty on development logins, which have no token.
    Token {
        token: String,
    },
    /// The answer to `ListWorlds`: the busiest loaded worlds, name and player count.
    Worlds {
        active: Vec<(String, u8)>,
    },
    /// Already cleaned by the server; draw it as given.
    Chat {
        from: PlayerId,
        text: String,
        kind: ChatKind,
    },
    /// A LiveKit token for this world's voice room (docs/knowledge/12). Sent after `Joined` and
    /// again before it expires. `expires_in` is in seconds.
    VoiceToken {
        url: String,
        token: String,
        expires_in: u32,
    },
    /// The world owner muted or unmuted `player`'s voice.
    VoiceMuted {
        player: PlayerId,
        muted: bool,
    },
}

/// What a player wears, slot by slot. Public: everyone in the world sees it.
pub type Equipped = [Option<ItemId>; SLOT_COUNT];

/// What a joining player receives. Other players arrive with body and outfit, without inventory or gems: those are private.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct WorldSnapshot {
    pub world: World,
    pub you: Player,
    /// Body, outfit and level of everyone else (their XP, inventory and gems stay private).
    pub others: Vec<(PlayerId, Body, Equipped, u16)>,
    /// Current damage (docs/knowledge/10: a joining client must see cracks already in progress).
    pub damage: HashMap<u32, Damage>,
    /// Display names of everyone in the world, the receiver included.
    pub names: Vec<(PlayerId, String)>,
    /// Country codes of those who gave one; set by the server after `of`.
    pub countries: Vec<(PlayerId, String)>,
}

impl WorldSnapshot {
    pub fn of(
        state: &WorldState,
        you: PlayerId,
        names: &HashMap<PlayerId, String>,
    ) -> Option<Self> {
        Some(Self {
            world: state.world.clone(),
            you: state.players.get(&you)?.clone(),
            others: state
                .players
                .values()
                .filter(|p| p.id != you)
                .map(|p| (p.id, p.body, p.equipped, p.level))
                .collect(),
            damage: state.damage.clone(),
            names: names.iter().map(|(&id, name)| (id, name.clone())).collect(),
            countries: Vec::new(),
        })
    }

    /// The client's copy of the world. The RNG is the server's alone, so the copy gets a dummy.
    pub fn into_state(self) -> WorldState {
        let mut world = self.world;
        (world.claim_of, world.world_claim) = world.claim_index_from_extras();
        let mut state = WorldState::new(world, 0);
        state.damage = self.damage;
        for (id, body, equipped, level) in self.others {
            state.join(id);
            if let Some(player) = state.players.get_mut(&id) {
                player.body = body;
                player.equipped = equipped;
                player.level = level;
            }
        }
        state.players.insert(self.you.id, self.you);
        state
    }
}

/// A chat line as it may be shown: control characters removed, whitespace trimmed, cut to
/// `MAX_CHAT_LEN` bytes on a character boundary, rude words masked. `None` if nothing is left.
pub fn clean_chat(text: &str) -> Option<String> {
    let mut clean = String::new();
    for c in text.trim().chars().filter(|c| !c.is_control()) {
        if clean.len() + c.len_utf8() > MAX_CHAT_LEN {
            break;
        }
        clean.push(c);
    }
    let clean = mask_profanity(clean.trim_end());
    (!clean.is_empty()).then_some(clean)
}

/// Words masked in chat (English and Indonesian). Matched on whole words, ignoring case and
/// repeated letters, so "fuuuck" is caught and "class" is not.
/// ponytail: a short fixed list; a maintained word list or per-world settings when it matters.
const PROFANITY: &[&str] = &[
    "fuck", "fucking", "shit", "bitch", "cunt", "dick", "pussy", "asshole", "bastard", "nigger",
    "faggot", "anjing", "anjir", "bangsat", "babi", "kontol", "memek", "ngentot", "goblok",
    "tolol", "bajingan", "jancok", "kampret",
];

/// `word` lower-cased with runs of the same letter collapsed ("Fuuuck" -> "fuck").
fn squash(word: &str) -> String {
    let mut out = String::new();
    for c in word.chars().flat_map(char::to_lowercase) {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

/// Names nobody may give a world: they would pass as official.
const RESERVED_WORLDS: &[&str] = &[
    "ADMIN",
    "ADMINS",
    "STAFF",
    "MOD",
    "MODS",
    "MODERATOR",
    "OWNER",
    "SERVER",
    "SYSTEM",
    "SUPPORT",
    "OFFICIAL",
    "GROWSOCIETY",
];

/// The server's deny-list on top of `validate_world_name`: reserved names, and names containing a
/// rude word anywhere (a name is one word, so whole-word matching would miss "FUCKWORLD").
/// ponytail: substring matching can refuse an innocent name; add exceptions when one is reported.
pub fn world_name_allowed(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let squashed = squash(name);
    game_shared::worldgen::validate_world_name(name)
        && !RESERVED_WORLDS.contains(&upper.as_str())
        && !PROFANITY
            .iter()
            .any(|word| squashed.contains(&squash(word)))
}

fn mask_profanity(text: &str) -> String {
    let banned: Vec<String> = PROFANITY.iter().map(|w| squash(w)).collect();
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if banned.contains(&squash(word)) {
            out.extend(std::iter::repeat_n('*', word.chars().count()));
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    // ponytail: a message type that cannot serialize is a programming error caught by the tests.
    postcard::to_stdvec(msg).expect("protocol types always serialize")
}

pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[derive(Debug)]
pub enum ReadError {
    Closed,
    TooLarge(usize),
    Malformed,
}

/// Writes one length-prefixed message.
pub async fn write_msg<T: Serialize>(
    stream: &mut quinn::SendStream,
    msg: &T,
) -> Result<(), quinn::WriteError> {
    let bytes = encode(msg);
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await
}

/// Reads one length-prefixed message, refusing anything over `max` bytes before allocating.
pub async fn read_msg<T: for<'a> Deserialize<'a>>(
    stream: &mut quinn::RecvStream,
    max: usize,
) -> Result<T, ReadError> {
    let mut len = [0; 4];
    stream
        .read_exact(&mut len)
        .await
        .map_err(|_| ReadError::Closed)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > max {
        return Err(ReadError::TooLarge(len));
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| ReadError::Closed)?;
    decode(&bytes).map_err(|_| ReadError::Malformed)
}

#[cfg(test)]
mod tests {

    #[test]
    fn world_names_deny_reserved_and_rude_names() {
        for name in ["START", "MYFARM", "W7", "CLASSROOM"] {
            assert!(world_name_allowed(name), "{name}");
        }
        for name in [
            "",
            "ADMIN",
            "staff",
            "FUCKWORLD",
            "MyShiiitFarm",
            "has space",
            "TOOLONGNAMEXXXXXXXXXXXXXX",
        ] {
            assert!(!world_name_allowed(name), "{name}");
        }
    }

    use super::*;
    use game_shared::items::DIRT;
    use game_shared::worldgen::generate;

    #[test]
    fn snapshot_roundtrips_and_rebuilds_the_claim_index() {
        let mut state = WorldState::new(generate("WIRE"), 3);
        let (me, other) = (PlayerId(4), PlayerId(9));
        state.join(me);
        state.join(other);
        state.players.get_mut(&other).expect("joined").gems = 999;
        let wings = game_shared::items::EQUIP_ID_BASE;
        state.players.get_mut(&other).expect("joined").equipped[6] = Some(ItemId(wings));
        let claim = game_shared::claims::Claim {
            owner: me,
            access: vec![],
            public: false,
            tiles: vec![5, 6],
        };
        state.world.insert_claim(5, claim);

        let msg = ServerMsg::Joined {
            snapshot: Box::new(
                WorldSnapshot::of(&state, me, &HashMap::from([(me, "me".into())])).expect("joined"),
            ),
        };
        let bytes = encode(&msg);
        assert!(bytes.len() < MAX_SERVER_MSG);
        let ServerMsg::Joined { snapshot } = decode(&bytes).expect("decodes") else {
            panic!("wrong variant");
        };
        let copy = snapshot.into_state();
        assert_eq!(copy.world, state.world);
        assert_eq!(copy.players[&me], state.players[&me]);
        // Someone else's gems never leave the server.
        assert_eq!(copy.players[&other].gems, 0);
        assert_eq!(copy.players[&other].body, state.players[&other].body);
        // What they wear is public.
        assert_eq!(
            copy.players[&other].equipped,
            state.players[&other].equipped
        );
    }

    #[test]
    fn names_are_short_and_plain() {
        assert!(valid_name("Asyra_7"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("é"));
        assert!(!valid_name(&"a".repeat(MAX_NAME_LEN + 1)));
        assert!(valid_country("id") && valid_country("england"));
        assert!(!valid_country("ID") && !valid_country("i") && !valid_country("../x"));
    }

    #[test]
    fn rude_words_are_masked_whole() {
        assert_eq!(
            clean_chat("what the FUUUCK, dasar Anjing!"),
            Some("what the ******, dasar ******!".into())
        );
        // Parts of other words are left alone.
        assert_eq!(
            clean_chat("classic shitake grass"),
            Some("classic shitake grass".into())
        );
        assert_eq!(clean_chat("hello"), Some("hello".into()));
    }

    #[test]
    fn chat_is_cleaned_and_bounded() {
        assert_eq!(clean_chat("  hi\u{7}\n there \t"), Some("hi there".into()));
        assert_eq!(clean_chat(" \n\u{0} "), None);
        let long = "é".repeat(MAX_CHAT_LEN);
        let cut = clean_chat(&long).expect("not empty");
        assert!(cut.len() <= MAX_CHAT_LEN && cut.chars().all(|c| c == 'é'));
        // The longest allowed line still fits a client message.
        let msg = ClientMsg::Chat {
            text: "x".repeat(MAX_CHAT_LEN),
        };
        assert!(encode(&msg).len() < MAX_CLIENT_MSG);
    }

    #[test]
    fn client_messages_fit_the_limit_and_garbage_is_refused() {
        let act = ClientMsg::Act {
            seq: u32::MAX,
            action: Action::Place {
                x: u32::MAX,
                y: u32::MAX,
                item: DIRT,
            },
        };
        assert_eq!(decode::<ClientMsg>(&encode(&act)).expect("decodes"), act);
        let hello = ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            auth: Auth::Token {
                token: "x".repeat(MAX_TOKEN_LEN),
            },
            country: Some("x".repeat(16)),
        };
        assert!(encode(&hello).len() < MAX_CLIENT_MSG);
        assert!(valid_password("12345678") && !valid_password("1234567"));
        assert!(!valid_password(&"x".repeat(MAX_PASSWORD_LEN + 1)));
        assert!(decode::<ClientMsg>(&[0xff, 0xff, 0xff]).is_err());
    }
}
