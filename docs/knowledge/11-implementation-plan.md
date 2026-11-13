# 11 · Implementation Plan

The build order for the scripting agent, and how QC reviews each step.

## Roles

| Role | Does | Does not |
|---|---|---|
| Scripting agent | Writes code and tests, one milestone at a time, following docs 02–10 | Change a **Rule** or **Reference** value, or skip a listed test, without asking QC |
| Research / QC agent | Maintains these docs, reviews each milestone against them, answers design questions | Write feature code |
| Owner | Answers the open decisions in the [README](README.md), approves commits and pushes | |

No QC during development: **QC happens once, at the end, when the owner asks for it** (owner's instruction,
2026-11-13). Finished work is reported to the owner, not reviewed step by step. File ownership between
sessions is in the [README](README.md#who-owns-which-files).

## Status (2026-11-13)

| Milestone | State | QC |
|---|---|---|
| M1 World model | Done | Passed |
| M2 See the world | Done | Passed |
| M3 A player that moves | Done; drawing later replaced by the coding session's rig | Passed (rig: static checks only) |
| M4 Punch, place, inventory | Done | Passed |
| M5 Farming and gems | Done | Passed |
| M6 Claims and the store | Done | **Not reviewed** |
| Camera zoom (coding session) | Done | Passed (static) |
| M7 Multiplayer | First step built (protocol, server, online client; see [10](10-networking-and-persistence.md) "As built"). Chat built (step 2). Trading, real auth, names, profanity filter to come | Not reviewed |
| Equipment (first pass) | Done: game_shared + HUD (scripting subagent); rig draws worn items, wings and the held pickaxe, snapshot and `PlayerJoined` carry `equipped` (coding session). Skin tone is still a local setting | Not reviewed |
| Voice chat V0–V4 | Built ([12](12-voice-chat.md) "As built"); tone-tested end to end, never heard by a person | Not reviewed |
| Commands C1–C2 | Built by the coding session ([13](13-commands-and-moderation.md)) | Not reviewed |
| Foliage | Built, client-side ([02](02-world.md#foliage-decoration)) | Not reviewed |

Nothing is committed to git yet. No interaction has been verified by a human click-through; every "Done" above rests
on unit tests, pipeline tests and still screenshots.

## Ground rules

1. One milestone at a time, in order. (QC itself now happens once at the end; see above.)
2. **Builds while working** (owner's rule, 2026-11-13; several sessions share one `target/` and wait on cargo's
   build lock):
   - Use `cargo check -p <crate you touched>` and `cargo test -p <that crate>` (or one test by name).
   - Do **not** run `cargo clippy --workspace --all-targets` or `cargo test --workspace` after each change.
   - Batch edits and build once, not after every small edit.
   - **One build at a time** across all sessions: before running cargo, check
     `ps aux | grep -E '[c]argo|[r]ustc'` and wait if another build is running.
   - Batch dependency changes (`Cargo.toml`) into one edit; add dependencies only when needed (a change makes Bevy
     recompile at opt-level 3).
   - Client work that does not touch voice builds with `--no-default-features` (skips LiveKit/WebRTC), or the
     coding session's `cargo dev` alias once it exists.
   - The full `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
     `cargo test --workspace` run **once at the end** (with the final QC) or right before the owner asks for a commit.
3. Every item in the milestone's "Tests that must exist" (in the linked doc) exists and passes.
4. Follow the code rules table in [09](09-architecture.md).
5. If a doc is wrong, unclear or contradicts another doc: stop and ask QC. Do not guess and do not silently diverge.
   If a doc is silent on a detail, pick the simplest option and list it in the report.
6. Bevy is pinned to 0.20. Verify API names against the 0.20 docs before using them.
7. Do not commit or push unless the owner asks. When committing: author is the repo identity only, with **no**
   `Co-Authored-By` trailer and no "Generated with" line.
8. No networking, server or database code before M7 is approved.

## Milestones

### M1 · World model (shared only, no rendering)

Docs: [02](02-world.md), [03](03-items-and-tiles.md)

- `rng.rs`, `items.rs` (starter table plus the recipe results from 06 may wait until M5), `world.rs`, `worldgen.rs`.
- Split `lib.rs` into modules; keep existing public names working.

Done when: the tests in 02 and 03 pass, and `game_shared` still has zero dependencies.

### M2 · See the world (client)

Docs: [02](02-world.md) rendering, [09](09-architecture.md) client structure and camera

- Replace the spinning square. `WorldState` resource built from `generate("START")`.
- `world_view.rs`: draw both layers, autotile masks for terrain materials, block sprites for the rest.
- `camera.rs`: free pan with arrow keys for now (the player arrives in M3), clamped to the world, nearest sampling.

Done when: running the client shows the generated world with correct grass edges, bedrock at the bottom, the main
door on the surface, and no visible seams while panning. Attach a screenshot to the report.

### M3 · A player that moves

Docs: [04](04-player-physics.md)

- `physics.rs` in shared with all listed tests.
- `player.rs`: keyboard to `Input`, `step` in `FixedUpdate`, render interpolation, animation states, sprite flip.
- Camera follows the player. Spawn at the main door.

Done when: the physics tests pass, and by hand the player walks, jumps exactly a 2-tile wall and not a 3-tile wall,
never enters a solid tile, and cannot leave the world. README vertical-slice items 1–6 are met.

### M4 · Punch, place, inventory

Docs: [05](05-punch-and-build.md), [08](08-inventory-and-trading.md) (inventory, drop, trash only), [09](09-architecture.md) contract

- `inventory.rs`, `rules.rs` with `Action::{Punch, Place, Drop, Trash}`, `apply`, drops and pickup.
- `interact.rs`: cursor to tile, highlight, click to Action. `hud.rs`: hotbar and item counts.
- The client applies Actions **only** through `apply` and updates sprites **only** from Events.

Done when: the tests in 05 and the inventory tests in 08 pass; by hand, blocks crack and break, drops can be picked
up, placed blocks appear with correct autotiling of neighbours, and range is enforced. README items 7–8 are met.

### M5 · Farming and gems

Docs: [06](06-farming-and-splicing.md), [07](07-economy-and-locks.md) (gems only)

- `farming.rs`, the full recipe table and its item rows, tree extras, growth-stage rendering, gem counter in the HUD.

Done when: the tests in 06 pass; by hand, a dirt seed grows in 31 s and yields its stored fruit, a splice produces
the right tree, and an invalid splice keeps the seed.

### M6 · Claims and the store

Docs: [07](07-economy-and-locks.md), [08](08-inventory-and-trading.md) (backpack upgrade)

- `claims.rs` with `can_edit` as the single permission function, the four claims, `Buy`, access/public actions,
  backpack upgrade, wrench outline of a claim's tiles, "locked" feedback.
- Since there is only one player, tests create a second `Player` in `WorldState` to act as the stranger.

Done when: the tests in 07 and the upgrade tests in 08 pass.

### M7 · Multiplayer (go-ahead given; built by the coding session)

Docs: [10](10-networking-and-persistence.md), [08](08-inventory-and-trading.md) trading

Scope as relayed from the owner: `crates/game_protocol`, `server/world_server` with one Tokio task per world,
`Hello`/`Join` and the snapshot, Events broadcast, movement predict and reconcile, the client connecting. Auth is a
development token for now; Argon2 login later. Offline play (no server address) must keep working exactly as
before. Trading and persistence follow.

Done when (for whoever reviews it):
- The server only calls `apply`, `step` and `collect_drops`; boundary checks (size, rate, sequence, identity from
  the connection) live in the server, rules stay in `game_shared`.
- Two clients in one world see each other move and see each other's punches and placements.
- A client that sends an out-of-range or forged action is rejected and stays in sync.
- `game_shared` without the `serde` feature still builds with zero dependencies.

## Report format (scripting agent → QC, at the end of each milestone)

```
Milestone: M_
Files added/changed: …
Tests added: … (count) · fmt / clippy / test: pass
Decisions made where docs were silent: …
Tuning values changed and why: …
Known gaps or shortcuts (ponytail comments): …
Questions for QC: …
```

## QC checklist (run by the research/QC agent on each report)

- [ ] The three cargo commands pass on a clean checkout
- [ ] Every "Tests that must exist" item for the milestone is present and actually asserts the rule (not a stub)
- [ ] No gameplay rule lives in `client/`; grep the client for range, cooldown, permission and cap logic
- [ ] `game_shared/Cargo.toml` has no `bevy` and no unexplained dependency
- [ ] No literal tuning numbers in systems; no item-id comparisons outside the named constants
- [ ] `can_edit` (from M6) has one implementation and every tile mutation reaches it
- [ ] No `unwrap`/`expect`/unchecked index on action-derived values in `game_shared`
- [ ] Rule and Reference values match the docs; changed Tuning values are reported
- [ ] The manual acceptance steps of the milestone were run in the real client
- [ ] Docs updated where the implementation legitimately differs
