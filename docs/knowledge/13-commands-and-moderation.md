# 13 · Commands, Admin and Moderation

Plan, not built. Chat commands for players, world owners, moderators and admins: moderation (mute, kick, ban),
item spawning, and world tools. Most of it needs the game server (M7) and accounts (Phase 7); the offline part can be
built now.

## Knowledge

### What the genre reference does (researched 2026-11-13)

The reference game's public command list splits powers into three layers. We reuse the layering, not the names.

| Layer | Examples in the reference | Notes |
|---|---|---|
| Any player | help, private message and reply, ignore, who is in this world, status, rules, report a world, list online moderators | Moderators cannot be ignored |
| World lock owner and access holders | pull a player to you, kick to the main door, ban from the world for one hour, unban all, kick everyone (with a cooldown) | Powers come from owning the land, not from a staff role |
| Moderators and developers | Not publicly documented; moderators' chat has its own colour | Staff powers are kept out of public docs |

Sources: [Commands (Growtopia Wiki)](https://growtopiawiki.com/w/Commands). Staff command syntax and punishment
lengths were not findable in public sources, so ours below are our own design.

Common practice from game-server administration guides: least privilege with a small number of roles, per-command
permission checks, no wildcard or self-elevation, every staff action logged, temporary punishments that expire on
their own, and two-factor login for staff accounts.

### Three kinds of authority

| Kind | Who | Scope | Comes from |
|---|---|---|---|
| **Player** | Everyone | Themselves | Being logged in |
| **World authority** | The world claim owner (full), and players on its access list (partial) | One world | The claim data that already exists ([07](07-economy-and-locks.md)) |
| **Staff role** | Moderator or Admin | Every world, the whole server | A role stored on the account, set by an Admin |

Staff roles are ordered: `Player < Moderator < Admin`. A staff member can never act on someone of equal or higher
role, and can never grant a role equal to or above their own. Nobody can raise their own role.

### Command list (ours)

Durations are written `30m`, `2h`, `7d` or `perm`. `<player>` is an exact player name (case-insensitive) or `#<id>`.
Commands that punish or create value require a reason, so the audit log is useful.

**Any player**

| Command | Does |
|---|---|
| `/help [command]` | Lists the commands you may use, or one command's syntax |
| `/who` | Players in this world |
| `/msg <player> <text>`, `/r <text>` | Private message, reply to the last one |
| `/ignore <player>`, `/unignore <player>` | Hide a player's chat and messages for you (staff cannot be ignored) |
| `/report <player> <reason>` | Files a report for moderators, with recent context attached automatically |
| `/staff` | Lists online moderators |

**World authority** (owner = world claim owner; access = on the world claim's access list)

| Command | Who | Does |
|---|---|---|
| `/kick <player>` | Owner, access | Sends the player to the main door |
| `/pull <player>` | Owner, access | Moves a player who is inside this world to you |
| `/wban <player> [duration]` | Owner | Bans from this world, default `1h`, max `7d`. The player is sent out of the world |
| `/wunban <player>` or `/wunban all` | Owner | Lifts world bans |
| `/vmute <player>` | Owner | Mutes the player's voice in this world ([12](12-voice-chat.md)) |

World authority never reaches staff: a world owner cannot kick, pull or ban a Moderator or Admin.

**Moderator**

| Command | Does |
|---|---|
| `/mute <player> <duration> <reason>` | Blocks chat and voice everywhere until it expires |
| `/unmute <player> <reason>` | Lifts a mute |
| `/warn <player> <reason>` | Shows the player a warning and records it |
| `/kickserver <player> <reason>` | Disconnects the player |
| `/ban <player> <duration> <reason>` | Account ban, up to `30d` for moderators |
| `/unban <player> <reason>` | Lifts a ban |
| `/goto <world>` / `/tp <player>` | Joins a world / moves to a player, ignoring world bans |
| `/bring <player>` | Moves a player to the moderator's world and position |
| `/spectate` | Toggles being invisible to players (still visible to staff) |
| `/inspect <player>` | Read-only view of a player's inventory, gems, sanctions and recent actions |
| `/history <player>` | Past warnings, mutes, bans and reports for a player |
| `/reports` | The open report queue |
| `/closeworld <world> <reason>` / `/openworld <world>` | Stops anyone but staff entering a reported world |

**Admin** (everything a Moderator can, plus)

| Command | Does |
|---|---|
| `/give <player> <item> [count] <reason>` | Puts items in a player's inventory, clamped to the 200 cap; reports how many actually fit |
| `/take <player> <item> [count] <reason>` | Removes items |
| `/spawn <item> [count]` | Drops the item at the admin's feet (for building and events; logged) |
| `/gems <player> <+n or -n> <reason>` | Adjusts gems, saturating at 0 and the `u32` maximum |
| `/setblock <x> <y> <item>` / `/clearblock <x> <y>` | Edits one tile in the current world, bypassing claims |
| `/ban <player> perm <reason>` | Permanent ban |
| `/role <player> <player\|moderator> <reason>` | Sets a role below Admin. Making an Admin is not a command; it is done in server configuration |
| `/announce <text>` | Message to every online player |
| `/shutdown <minutes> <reason>` | Announces, then stops the server cleanly after saving |

`<item>` is an item id or its exact table name (`planks`, `small claim`); seeds are `<name> seed`.

### Rules every command follows

| Rule | Why |
|---|---|
| The server decides, always. The client may hide commands you cannot use, but the server checks the role on every command | A modified client can send anything |
| Your role comes from your account on the server, never from anything the client sends | Same as identity in [10](10-networking-and-persistence.md) |
| Commands that change the world produce the same Events as normal play | Every client stays in sync; no special sync path |
| Every staff command and every world-owner punishment is written to an append-only audit log: who, their role, target, arguments, reason, world, time, result (including refusals) | Staff abuse and mistakes must be traceable |
| Sanctions are stored with an expiry and checked where they bite: account ban at login, mute at chat and voice, world ban at world join | Temporary punishments end without anyone remembering to lift them |
| The punished player is told what happened, for how long and why | Silent punishment reads as a bug and breeds tickets |
| Commands are rate-limited, staff included | A runaway script or a compromised staff account is slowed down |
| No command runs arbitrary code, reads files, or changes server configuration | Keeps a stolen staff account from becoming a stolen server |
| Spawned value is always logged with a reason | Item creation is the most abusable power in an economy game |

### Offline mode

Single-player (no server) treats the local player as **Admin** for world-changing commands: `/give`, `/spawn`,
`/gems`, `/setblock`, `/clearblock`, plus `/help`. This makes testing and QC much faster (spawn claims, seeds and
gems instead of farming for them). Moderation commands do nothing offline. An online client never trusts this: the
server ignores anything the client thinks its role is.

## Technical

### Where the code lives

| Part | Crate | Why there |
|---|---|---|
| `Command` enum, `parse(text) -> Result<Command, ParseError>`, duration parsing, item-name lookup, `required(&Command) -> Requirement` | `game_shared::commands` (new) | Pure and testable; the client uses the same parser for `/help`, autocomplete and early error messages |
| World-changing commands (`give`, `take`, `spawn`, `gems`, `setblock`, `clearblock`, `kick`, `pull`) | `game_shared::rules::apply_admin(state, by, cmd, now) -> Result<Vec<Event>, Reject>` | Same contract as `apply`: validate first, then write, every change an Event ([09](09-architecture.md)) |
| Roles, sanctions, reports, audit log, ban/mute/role/announce/shutdown, world bans | `server/world_server` | They touch accounts and sessions, which only the server has |
| Chat line and command input | `client/game_client/src/chat.rs` (exists, owned by the coding session) | Its single `submit` function is where the client parses `/` lines (for `/help`, autocomplete, early errors) |
| Server entry point | `Room::command` in `server/world_server/src/world.rs` (exists, owned by the coding session) | Every `/` line already arrives here; the parser, role check and dispatch drop in at this one function |

`apply_admin` takes the **already authorized** caller: the server (or offline mode) checks `required(&cmd)` against
the caller's role and world authority before calling it. Keeping the check outside the function keeps
`game_shared` free of account data; keeping the mutation inside keeps it the only writer.

### Types

```rust
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum Role { Player, Moderator, Admin }

pub enum Requirement { Anyone, WorldAccess, WorldOwner, Role(Role) }

pub enum Duration { Minutes(u32), Permanent }   // parsed from 30m / 2h / 7d / perm, with per-role maximums

pub enum Target { Name(String), Id(PlayerId) }

pub enum Command {
    Help { topic: Option<String> }, Who, Msg { to: Target, text: String }, Reply { text: String },
    Ignore { who: Target, on: bool }, Report { who: Target, reason: String }, Staff,
    Kick { who: Target }, Pull { who: Target }, WorldBan { who: Target, length: Duration },
    WorldUnban { who: Option<Target> }, VoiceMute { who: Target },
    Mute { who: Target, length: Duration, reason: String }, Unmute { who: Target, reason: String },
    Warn { who: Target, reason: String }, KickServer { who: Target, reason: String },
    Ban { who: Target, length: Duration, reason: String }, Unban { who: Target, reason: String },
    Goto { world: String }, Tp { who: Target }, Bring { who: Target }, Spectate,
    Inspect { who: Target }, History { who: Target }, Reports,
    CloseWorld { world: String, reason: String }, OpenWorld { world: String },
    Give { who: Target, item: ItemId, count: u16, reason: String },
    Take { who: Target, item: ItemId, count: u16, reason: String },
    Spawn { item: ItemId, count: u16 }, Gems { who: Target, delta: i64, reason: String },
    SetBlock { x: u32, y: u32, item: ItemId }, ClearBlock { x: u32, y: u32 },
    SetRole { who: Target, role: Role, reason: String },
    Announce { text: String }, Shutdown { minutes: u32, reason: String },
}

pub struct Sanction {
    pub kind: SanctionKind,          // Ban, Mute, WorldBan { world }, Warning
    pub target: PlayerId,
    pub by: PlayerId,
    pub reason: String,
    pub created: f64,
    pub until: Option<f64>,          // None = permanent (or not applicable, for warnings)
}
```

Text limits at the parser: a command line at most 256 bytes, a reason at most 200, control characters stripped,
counts `1..=200`, coordinates must be inside the world. A malformed command is a `ParseError` with a one-line usage
hint, never a panic.

### Messages (extends [10](10-networking-and-persistence.md))

```rust
ClientMsg::Chat { text: String }                 // plain chat or a /command; the server parses
ServerMsg::Chat { from: PlayerId, text: String, kind: ChatKind }   // World, Private, Staff, System
ServerMsg::CommandResult { ok: bool, text: String }
ServerMsg::Sanctioned { kind: SanctionKind, until: Option<f64>, reason: String }
```

World-changing results arrive as ordinary `Events`.

### Storage (Phase 7)

| Table | Holds |
|---|---|
| `account_roles(player_id, role, granted_by, granted_at)` | Staff roles; no row means Player |
| `sanctions(id, kind, target, by, reason, created, until, lifted_by, lifted_at)` | Never deleted; lifting sets `lifted_*` |
| `world_bans(world, target, by, until)` | Per-world bans |
| `reports(id, reporter, target, world, reason, context, created, status, handled_by)` | `context` = the last 20 chat lines in that world and both players' positions, captured when filed |
| `audit_log(id, at, actor, actor_role, command, target, args, reason, world, result)` | Append-only; the server's database user has no `UPDATE` or `DELETE` on it |

Until persistence exists, the server writes the audit log as one JSON line per entry to a file and keeps sanctions
in memory (lost on restart; acceptable for development only).

### Phases

| Phase | Delivers | Needs | Can start now? |
|---|---|---|---|
| C1 | `game_shared::commands` parser, `apply_admin` for give/take/spawn/gems/setblock/clearblock, chat line in the client, offline Admin | Nothing | **Yes**, offline |
| C2 | Chat over the network, `/who`, `/msg`, `/ignore`, server role check, world-authority commands (kick, pull, wban, wunban) | M7 | After M7's join flow |
| C3 | Moderator and Admin commands, audit log file, in-memory sanctions, sanction messages to players | C2 | After C2 |
| C4 | Sanctions, roles, reports and audit log in the database; report queue; 180-day claim inactivity | Phase 7 persistence | Later |
| C5 | Staff two-factor login, a web view of reports and audit log | C4, accounts | Later |

### Tests that must exist

- Parser: every command parses from its documented syntax; bad syntax gives `ParseError` with usage; durations
  `30m`/`2h`/`7d`/`perm` parse and anything else fails; counts outside `1..=200`, coordinates outside the world and
  over-long text are rejected; item names resolve case-insensitively and unknown names fail.
- `required()` gives the right requirement for every command (one table-driven test).
- `apply_admin`: `/give` past the cap adds only what fits and says so; `/take` more than held takes what is there or
  fails as documented; `/gems` saturates at both ends; `/setblock` on the main door or out of bounds is refused;
  every change comes back as Events; a refused command leaves the state unchanged (same pattern as `apply`).
- Authority: a Moderator cannot act on an Admin or another Moderator; nobody can set a role equal to or above their
  own; a world owner cannot kick or ban staff; an access holder cannot `/wban`.
- Sanctions: an expired ban lets the player log in; a mute blocks chat and voice; a world ban blocks joining that
  world only.
- Audit: every staff command, including refused ones, produces exactly one audit entry.

## Open decisions for the owner

| # | Question | Default in this plan |
|---|---|---|
| C-D1 | Role names and levels: is `Player / Moderator / Admin` enough, or is a helper/trial-moderator role wanted? | Three roles |
| C-D2 | Do access holders get `/kick` and `/pull`, or only the world owner? | Access holders get kick and pull, not ban |
| C-D3 | Should admin-spawned items be marked as spawned (visible in trades, excluded from economy statistics)? | Not marked; the audit log is the record |
| C-D4 | Offline mode gives the local player Admin commands. Keep it in release builds, or only in debug builds? | Debug builds only |
| C-D5 | Maximum moderator ban length | `30d`; permanent bans are Admin only |
| C-D6 | Who builds C1 and C2? | **Decided:** the coding session, at the owner's request (2026-11-13) |
