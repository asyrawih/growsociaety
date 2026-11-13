# 10 · Networking and Persistence

Design for Phases 5 and 7. M7 (Phase 5: protocol, server, client connection) is being built by the coding session as
of 2026-11-13, with a development token instead of real login. Persistence (Phase 7) is not started.

### Gaps the single-player code leaves for M7

| Gap | Today | What M7 needs |
|---|---|---|
| Time | `now` is seconds since the client app started; trees store it in `planted_at` | One server clock for `apply`; clients draw growth using `now + offset` from `Welcome.server_time` |
| RNG | Fixed `RULES_SEED` constant in the client | The server seeds each world's `Rng`; clients never roll |
| Damage and cooldown | Not in Events (documented exemptions to contract rule 3, see [09](09-architecture.md)) | The join snapshot carries the current `damage` map |
| Player id | `LOCAL_PLAYER` constant | Assigned by the server from the connection; never read from a message body |

## Knowledge

### Server authority

The server owns the truth: tiles, inventories, gems, positions, time. A client is a remote control with a screen.

| The client may | The client may not |
|---|---|
| Send inputs and Actions | Send results ("I broke this", "I now have 50") |
| Predict its own movement for responsiveness | Decide its own position |
| Draw whatever it wants | Be believed about range, cooldown, ownership, counts or time |

Assume every client is modified. In this genre the common cheats are: reach hacks (punching far tiles), speed and
fly hacks, cooldown skipping, item duplication through trade or drop races, editing protected tiles, and forged
quantities. Each one is answered by a check that already lives in `apply` or `step`, which is why those functions
are in `game_shared` and run on the server.

### One world, one task

Each loaded world is one Tokio task that owns its `WorldState` and processes Actions from a queue, one at a time.

- No locks around world state, and no two Actions ever interleave. Duplication races inside a world become
  impossible by construction, not by careful coding.
- A world with no players is saved and unloaded.
- A world is 20 KB of tiles; a server holds thousands in memory.

### What is sent

| When | Message | Size idea |
|---|---|---|
| Player enters | Full world: tiles, extras, drops, players | ~20 KB raw, far less compressed |
| Something changes | The `Event`s from `apply`, to everyone in the world | Tens of bytes |
| 20 times a second | Positions of the other players | ~10 bytes each |
| Client → server, each tick with input | `Input` + sequence number | A few bytes |
| Client → server, on click | `Action` + sequence number | A few bytes |

Tree growth is never sent after planting: clients compute it from `planted_at` and a clock offset received at join.

### Movement: predict and reconcile

1. The client runs `physics::step` immediately on its own input and remembers each input with a sequence number.
2. The server runs the same `step` on the same inputs and replies with the authoritative `Body` and the last
   sequence it processed.
3. The client snaps to that body and replays the inputs it sent after that sequence.

Because both sides run the identical deterministic function, corrections are invisible unless the client cheated or
the world changed under it. Other players are drawn about 100 ms in the past, interpolated between snapshots.

## Technical

### As built (first networking step, 2026-11-13)

Reported by the coding session; not reviewed by QC. Where this differs from the design below, this wins.

| Topic | As built |
|---|---|
| Framing | `postcard`, length-prefixed messages on **one** QUIC bidirectional stream; `Bodies` as datagrams |
| Server | One Tokio task per world; `Hello` protocol-version check; `PlayerId` from the connection; size checked before decode; increasing `seq`; token buckets (`Input` 60/s, `Act` 10/s); repeated violations earn strikes, then a kick |
| Replies | `Rejected` became `Acted { seq, result }`: the acting player's full answer (its events or its reject). Those events are **not** repeated to that player in `Events` |
| Presence | `PlayerJoined` / `PlayerLeft` messages added |
| Privacy | Private events (inventory, gems, backpack) go only to their owner |
| Auth | Accounts with Argon2id password hashes in PostgreSQL (`DATABASE_URL`, migrations run at server start; demo accounts `demo1`..`demo10` seeded). `Hello` carries `auth: Auth::{Login, Register, Token, Dev}`. PROTOCOL_VERSION is 7 |
| Certificate | Self-signed, persisted across restarts as `world_server_cert.der` and pinned by the client |
| Clock | The client measures its offset to the server clock once, at `Welcome` |
| Unloading | A world with no players is dropped (no persistence yet) |
| Chat | `ClientMsg::Chat { text }`, `ServerMsg::Chat { from, text, kind: World \| System }`. `clean_chat`: trimmed, control characters stripped, at most `MAX_CHAT_LEN = 256` bytes cut on a character boundary, empty refused. Rate limit 1 line/s with a burst of 5; refused lines count as strikes. Broadcast to the whole world, speaker included. Every line starting with `/` goes to one server function, `Room::command` (`server/world_server/src/world.rs`), which for now answers "Commands are not available yet." Client: Enter opens the line, Enter sends, Esc cancels; the last 6 lines show bottom-left for 10 s, 50 lines of history while open; System lines are amber; game keys are ignored while typing. Offline, lines show locally only. Not yet: profanity filter, player names (lines say "Player N"), mute |
| Client | `GAME_SERVER=localhost` turns on online mode: blocking join, client state mirrored with `game_shared::mirror::apply_event`, prediction and reconcile in `player.rs`, remote players drawn from server bodies. Offline is unchanged without it |
| Tests | Real-QUIC integration test: two clients, movement, a refused far punch, a replayed `seq` ignored, `PlayerLeft` |

How to run it is in the root `README.md`.

### Lobby and login (as built, reported by growsociaety-75)

- `client/game_client/src/lobby.rs` is a small separate Bevy App that runs before the game when online and no
  credentials are in the environment (`GAME_AUTH_TOKEN` / `GAME_TOKEN` / `GAME_USER`). Offline and dev runs skip it.
  The server address comes from `net::server_address()`: `GAME_SERVER`, otherwise the production server in release
  builds.
- Two screens: **Login** (name, password with a show/hide toggle, "Remember me" keeps only the name in
  `<config dir>/lobby_name.txt`, since tokens live 15 minutes) and **Register** (name, password, confirm). Input is
  checked locally first.
- Login and register go **over QUIC**, not HTTP: a short connection sends `Hello` with `Auth::Login` or
  `Auth::Register` (PROTOCOL_VERSION 7). On `Welcome` the lobby takes the signed `token` (15 minutes), closes the
  connection cleanly and re-executes the game with `GAME_AUTH_TOKEN` set (exec on Unix, spawn and exit elsewhere), so
  the password never reaches the environment. `net.rs` then sends `Hello { auth: Auth::Token }`. A `Kicked` reason is
  shown in the panel with `sfx/denied.ogg`. The window closes and reopens once.
- `bgm/lobby.ogg` loops at the stored music volume (`audio::music_player`); silent in screenshot mode.

### Messages (`crates/game_protocol`, Phase 5)

```rust
enum ClientMsg {
    Hello { token: String, protocol: u16 },
    Join { world: String },
    Leave,
    Input { seq: u32, input: Input },
    Act { seq: u32, action: Action },
    Chat { text: String },
}

enum ServerMsg {
    Welcome { player: PlayerId, server_time: f64 },
    Joined { snapshot: WorldSnapshot },
    Events { events: Vec<Event> },
    Rejected { seq: u32, reason: Reject },
    Bodies { ack_seq: u32, you: Body, others: Vec<(PlayerId, Body)> },
    Kicked { reason: String },
}
```

`Action`, `Event`, `Reject`, `Input` and `Body` are the `game_shared` types with `serde` derives. The protocol crate
adds only the envelope. `protocol: u16` is bumped on any incompatible change and mismatches are refused at `Hello`.

### Transport (decision D4: decided)

**QUIC via `quinn`.** A hand-written UDP protocol would have to provide a handshake, reliability and ordering,
unreliable delivery, fragmentation for the 20 KB snapshot, congestion control, encryption and DoS resistance. QUIC
provides all of them.

| Traffic | QUIC channel | Why |
|---|---|---|
| `Hello`, `Join`, `Act`, `Events`, `Rejected`, `Joined` snapshot | Reliable, ordered stream | Losing or reordering a world change desyncs the client |
| `Input` (client → server), `Bodies` (server → client) | Unreliable datagrams | Only the newest matters; a late one is useless |

Datagrams have a size ceiling set by the path MTU (roughly 1,200 bytes is the safe assumption). A `Bodies` message
for a full world (64 players × ~10 bytes) fits; anything that might not fit goes on the stream.

**Serialization: `postcard`** through `serde`. `game_shared` gets `serde` as an **optional** dependency behind a
`serde` feature, with `#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]` on its plain data types.
The default build of `game_shared` stays dependency-free; only the protocol crate, the server and the client enable
the feature.

What is **not** serialized:

| Data | Why |
|---|---|
| `ItemDef`, `ITEMS`, `RECIPES`, `CLAIMS` | Static tables compiled into both sides; only `ItemId`s cross the wire |
| `World.claim_of`, `World.world_claim` | Derived from `extras`; rebuilt after decoding a snapshot, never trusted from the wire |

Certificates: a self-signed certificate generated at server start is fine for development. The client must not
skip verification in a release build; pin the dev certificate or use a real one before anyone outside the team
connects.

### Validation at the boundary (server, per message)

| Check | Detail |
|---|---|
| Size and shape | Reject oversized or undecodable packets before allocating. World names, chat text and offer counts have hard length/range limits |
| Rate | Per-connection token bucket per message type (e.g. at most 10 `Act` and 60 `Input` per second). Excess is dropped, repeated excess disconnects |
| Sequence | `seq` must increase. Old or duplicate numbers are ignored, which also stops replays |
| Identity | The `PlayerId` comes from the authenticated connection, **never** from the message body |
| Rules | Everything else is `apply` returning a `Reject` |
| Movement | The server's own `step` result is the position. A client that claims otherwise is simply corrected |
| Text | Chat and sign text: length limit, control characters stripped, profanity filter, rate limit |

Never trust that a field is in range because the official client always sends it in range.

### Authentication

Minimum for Phase 5: the client logs in over HTTPS, receives a short-lived signed token, and presents it in `Hello`.
Passwords are hashed with a memory-hard algorithm (Argon2id) by a maintained library; never store or log them.
Do not design account recovery, purchases or moderation tooling until the owner asks.

### Persistence (Phase 7)

| Data | Store | Format |
|---|---|---|
| Account, gems, inventory | PostgreSQL rows | `inventory(player_id, item_id, count)` with `CHECK (count BETWEEN 1 AND 200)` and a primary key on `(player_id, item_id)` |
| World | PostgreSQL, one row per world | Versioned binary blob: header (`version: u16`, width, height), tiles, extras, drops |
| Derived data (`claim_of`, damage) | Not stored | Rebuilt or dropped on load |

Rules:

- **The database enforces the invariants too.** The `CHECK` and the primary key mean a logic bug cannot persist 201
  of an item or two rows for one item type.
- **Anything that moves value between two owners is one transaction**: a trade's two inventories; a purchase's gems
  and item; a world save together with the inventory changes caused by Actions since the last save (or accept that
  a crash rolls both back to the same point, which is the simpler and recommended approach: save world and its
  players' inventories together, on a timer and on unload).
- **The world blob is versioned from the first byte.** Loading an old version runs a migration function. Item ids
  never change meaning ([03](03-items-and-tiles.md)), so old worlds stay valid.
- **Crash recovery** means: on restart, every world and inventory is at its last consistent save. No item exists in
  two places and none is lost beyond the save interval.

Redis (listed in the root README's stack) is **not used**: sessions and world-to-server routing across several server
processes live in PostgreSQL lease tables instead (see "Scaling" below).

### Scaling (decided 2026-11-13; written by the session handling scaling, growsociaety-75)

At most **500 players per server process** (`MAX_PLAYERS` in `server/world_server/src/conn.rs`; a full server
answers `Hello` with `Kicked { "server full" }`). More players means more server processes, all sharing **one**
PostgreSQL. Ownership lives in Postgres lease tables (`server/world_server/migrations/0001_leases.sql`), not Redis:
`world_lease` (a world runs on one server; others redirect), `player_session` (an account is online once),
`game_server` (live servers and load, for login to pick one under 500). Leases expire after 30 s unless renewed
every 10 s, so a crashed server's worlds free themselves. Not yet wired: needs the database pool from the login work.
Sizing per server: 2–4 vCPU, 4 GB, at least a 100 Mbps port (worst case ~50 Mbps out at 500 players).

### What earlier milestones must respect so this stays cheap

1. All mutation through `apply`, all movement through `step` ([09](09-architecture.md)).
2. No system clock and no global RNG in `game_shared`.
3. Rendering driven by `Event`s, not by the code that issued the Action.
4. Plain data types in `game_shared` (no references, no trait objects in `Action`/`Event`/`World`) so `serde` derives
   are a one-line addition.
5. Stable numeric ids for items.
