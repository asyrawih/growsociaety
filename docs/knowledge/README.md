# Project Knowledge Base

Everything a contributor (human or agent) needs to build this game: what it is, how each mechanic works, and how
the code is laid out. Read `01` first, then the doc for the system you are touching, then `11` for the task order.

## Index

| # | Doc | Type | What it answers |
|---|---|---|---|
| 01 | [Game overview](01-game-overview.md) | Knowledge | What the game is, the core loop, what is original and what is borrowed |
| 02 | [World](02-world.md) | Knowledge + technical | World size, layers, generation, tile storage |
| 03 | [Items and tiles](03-items-and-tiles.md) | Knowledge + technical | Item table, ids, collision kinds, rarity |
| 04 | [Player physics](04-player-physics.md) | Knowledge + technical | Movement, jump, gravity, AABB collision |
| 05 | [Punch and build](05-punch-and-build.md) | Knowledge + technical | Breaking, placing, range, drops |
| 06 | [Farming and splicing](06-farming-and-splicing.md) | Knowledge + technical | Seeds, trees, grow time, recipes |
| 07 | [Economy and locks](07-economy-and-locks.md) | Knowledge + technical | Gems, claims, ownership, permissions |
| 08 | [Inventory and trading](08-inventory-and-trading.md) | Knowledge + technical | Slots, stack cap, trade state machine |
| 09 | [Architecture](09-architecture.md) | Technical | Crates, the Action/Event contract, Bevy plugins |
| 10 | [Networking and persistence](10-networking-and-persistence.md) | Technical | Server authority, messages, validation, storage |
| 11 | [Implementation plan](11-implementation-plan.md) | Technical | Milestones, acceptance checks, QC process |
| 12 | [Voice chat](12-voice-chat.md) | Knowledge + technical | LiveKit rooms per world, proximity voice, tokens, moderation, phases |
| 13 | [Commands and moderation](13-commands-and-moderation.md) | Knowledge + technical | Chat commands, roles, world-owner powers, mute/ban, item spawning, audit log |
| 14 | [Levels and XP](14-levels.md) | Knowledge + technical | XP sources, level curve, level-up event, name colour |
| 15 | [Mobile (iOS and Android)](15-mobile.md) | Plan | What carries over, what breaks today, touch controls, permissions, store rules, phases |
| 16 | [Auto-update](16-auto-update.md) | Plan | Server version gate, desktop self-update (Velopack), store updates, signing, release pipeline |
| 17 | [Growtopia reference](17-growtopia-reference.md) | Research | How the genre reference's systems work (player mods, pickaxes, wings, item categories) and takeaways; structures only, never content |
| 18 | [Crafting and item mods](18-crafting-and-mods.md) | Knowledge + technical | Mods by slot (extra jump, glide, run, dig, reach, gems), pickaxe ladder to 2×, Workbench and starter recipes |
| ux/ | [UX specs](ux/) | UI/UX | One-page spec per new UI or feature, written by the UI/UX session (growsociaety-64) **before** implementation (owner's rule, 2026-11-14). Implement from the spec; ask 64 about anything it does not cover |

Art rules live separately in [`../art/STYLE.md`](../art/STYLE.md) and [`../art/tileset-terrain.md`](../art/tileset-terrain.md).

## How to read the numbers

Every number in these docs carries one of three labels:

| Label | Meaning | Can a scripter change it? |
|---|---|---|
| **Rule** | A design decision the mechanic depends on (e.g. stack cap 200, rarity is additive) | No, raise it with QC first |
| **Reference** | Verified from the genre reference during research (2026-11-13); kept as our starting value | Only with a reason |
| **Tuning** | A starting guess with no verified source. Expected to change during playtests | Yes, keep it a named constant |

All tunables must be named constants in `game_shared`, never literals scattered in systems.

## Glossary

| Term | Meaning |
|---|---|
| Tile | One 32×32 grid cell. Has a foreground item and a background item |
| Foreground (fg) | The block layer the player collides with |
| Background (bg) | The wall layer behind the player. Never collides |
| Item | Anything with an `ItemId`: blocks, backgrounds, seeds, tools, claims |
| Rarity | An item's progression score. Drives grow time and gem drops |
| Splice | Planting a second, different seed on a sapling to produce a new item's tree |
| Claim | A lock block that gives its owner exclusive edit rights over some tiles |
| Action | A request to change the world (`Punch`, `Place`, …). May be rejected |
| Event | A fact about a change that happened (`TileChanged`, `ItemDropped`, …) |

## Decisions

Open (need the owner's answer; the code runs on the default):

| # | Question | Default in use | Where |
|---|---|---|---|
| D1 | World height: keep 100×50 (current code) or move to 100×60 like the reference? | Keep 100×50, layout derived from constants | [02](02-world.md) |
| D2 | Player hitbox 20×44 px (matches the 1.5-tile sprite, needs 2-tile-high tunnels) or ≤30 px tall (fits 1-tile tunnels)? | 20×44 | [04](04-player-physics.md) |
| D3 | Item definitions as a Rust const table or a data file (RON/JSON)? | Const table until a non-programmer needs to edit it | [03](03-items-and-tiles.md) |
| D5 | Chunked world storage (README) for a 5,000-tile world? | Flat `Vec<Tile>`; chunk only if profiling demands | [02](02-world.md) |

Decided:

| # | Decision | Source | Where |
|---|---|---|---|
| D4 | Transport is QUIC via `quinn` (reliable streams for actions, events and the snapshot; unreliable datagrams for input and bodies). Serialization is `postcard` through optional `serde` derives in `game_shared` | The owner, relayed by the coding session on 2026-11-13 | [10](10-networking-and-persistence.md) |

## Who owns which files

Several Claude sessions work in this one working tree. Edit only what your session owns; ask the owner's session
before touching anything else. Shared builds can leave stale incremental objects: if linking fails with undefined
symbols after another session built, `cargo clean -p <crate>` and rebuild.

| Area | Owner |
|---|---|
| `docs/knowledge/**` except `ux/` | Research/QC session |
| `docs/knowledge/ux/**` (UX specs) | UI/UX session (growsociaety-64) |
| `crates/game_shared/**` (rules, items, world, farming, claims, …) | Scripting subagent of the research/QC session. The optional `serde` feature and its derives: coding session |
| `client/game_client/src/{world_view,hud,interact,screenshot}.rs` | Scripting subagent of the research/QC session |
| `client/game_client/src/almanac.rs`, `hints.rs` | Client-UI/docs session (growsociaety-8e) and its subagents, since 2026-11-14 (were growsociaety-f0's) |
| `client/game_client/src/player.rs`, `player/rig.rs`, `camera.rs` | Coding session (rig, equipment, zoom) |
| `client/game_client/src/{main,state}.rs` | Coding session for network wiring (`mod net;`, routing actions to the server); scripting subagent for everything else, after asking |
| `client/game_client/src/net.rs`, `chat.rs`, `crates/game_protocol/**`, `server/**`, root `Cargo.toml` | Coding session (multiplayer, M7, chat) |
| `client/game_client/src/lobby.rs` (login/register lobby, a separate App that re-execs the game) | growsociaety-75 |
| Commands (doc 13, C1 + C2): `crates/game_shared/src/commands.rs`, `rules::apply_admin`, the command launcher, offline routing in `state.rs`, `Room::command` | Coding session (taken over 2026-11-13 at the owner's request) |
| `assets/**`, `docs/art/**` | Asset session (PixelLab) |
| `infra/livekit/` except `prod/` (local development LiveKit) | Research/QC session |
| `infra/game/`, `infra/livekit/prod/`, production servers (`76.13.196.13`: game server, LiveKit at `wss://livekit.kelasmalam.app`) | Deployment session (growsociaety-f8) |

## Originality rule

This is an original game in the tile-sandbox-MMO genre. We reuse **mechanic structures** (additive rarity, cubic grow
time, tiered claims, a per-item stack cap). We do **not** reuse another game's name, item names, recipe list, art,
sounds, UI layout or world names. Every recipe and item name in these docs is ours; keep it that way when adding more.
