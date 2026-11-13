# 09 · Architecture

## The one idea

**All gameplay rules live in `game_shared` as plain functions. The client and the server are both thin shells
around them.**

```
                 ┌──────────────────────── game_shared (no Bevy, no IO, no clock) ───────────────────────┐
                 │  items · world · worldgen · physics::step · rules::apply · inventory · farming · claims │
                 └───────────────▲──────────────────────────────────────────────────▲─────────────────────┘
                                 │                                                  │
        client/game_client (Bevy)│                                 server/world_server (Tokio, Phase 5)
        input → Action           │                                 socket → Action
        apply() locally (now)    │                                 apply() authoritatively
        Events → sprites, sound  │                                 Events → broadcast + persist
```

Today the client calls `apply` itself and the game is single-player. In Phase 5 the same call moves to the server
and the client sends the `Action` over the network instead. Nothing in the rules changes, and nothing in the
client's rendering changes, because rendering already reacts to `Event`s and not to its own guesses.

This is the reason for every "must be in `game_shared`" and "must go through `apply`" in the other docs.

## The Action / Event contract

```rust
pub struct WorldState {
    pub world: World,
    pub players: HashMap<PlayerId, Player>,
    pub damage: HashMap<u32, Damage>,
    pub rng: Rng,
}

pub struct Player {
    pub id: PlayerId,
    pub body: Body,
    pub inventory: Inventory,
    pub gems: u32,
    pub last_punch: f64,
}

pub fn apply(state: &mut WorldState, player: PlayerId, action: Action, now: f64)
    -> Result<Vec<Event>, Reject>
```

Contract rules:

1. **`apply` is the only function that mutates `WorldState`** in response to a player. (`physics::step` mutates a
   `Body`; world generation builds a `World`. Nothing else writes.)
2. **Validate, then mutate.** All checks run before the first write. A rejected action leaves the state exactly as
   it was. There is no partial success.
3. **Events are complete.** A client that applies the returned events to its copy of the state ends up identical to
   the server. If a change is not in an event, it is a bug. Two pieces of transient state are exempt, because the
   client never needs the server's copy: a player's `last_punch` (the server enforces the cooldown; the client only
   paces its own sends) and the lazy expiry of tile damage (the client hides cracks with its own timer). The join
   snapshot still carries current damage.
4. **No clock, no IO, no globals.** Time comes in as `now`, randomness from `state.rng`. Same state plus same inputs
   gives the same result, which is what makes rules testable without a window or a socket.
5. **`Reject` is a value, not a panic.** Bad input from a player must never crash. `unwrap`/`expect`/indexing on
   anything derived from an action is not allowed in `game_shared`.

## Workspace layout

```
crates/game_shared/src/
  lib.rs         constants, coordinate helpers, re-exports
  rng.rs         SplitMix64
  items.rs       ItemId, ItemDef, ITEMS, item()
  world.rs       Tile, World, TileExtra, get/set
  worldgen.rs    generate(name), validate_world_name
  physics.rs     Body, Input, step
  inventory.rs   Inventory
  farming.rs     Tree, RECIPES, grow_seconds, splice
  claims.rs      Claim, can_edit, claim area
  rules.rs       Action, Event, Reject, WorldState, apply

client/game_client/src/
  main.rs        App setup, plugin list
  state.rs       WorldState resource, Action/Event messages, the one apply system
  world_view.rs  tile, tree and drop sprites, autotile masks, reacts to Events
  player.rs      input → Input, FixedUpdate step, interpolation
  player/rig.rs  code-driven character rig, clothing layers, skin recolour
  camera.rs      smooth follow, clamped to world bounds, zoom 1x–4x
  interact.rs    cursor → tile, click → Action, tile highlight, wrench/claim panel
  hud.rs         hotbar, gems, inventory grid, store, messages, "pointer over UI"
  screenshot.rs  GAME_SCREENSHOT / GAME_SCREENSHOT_UI debug capture for QC
  net.rs         connection to the server (M7)

server/world_server/   one Tokio task per world, boundary checks (M7)
crates/game_protocol/  ClientMsg / ServerMsg envelopes over the game_shared types (M7)
```

Rules for the layout:

- Create a file when its milestone starts, not before (README: no empty stubs).
- `lib.rs` currently holds everything in 30 lines. Split it as modules arrive; keep existing public names
  (`WORLD_WIDTH`, `tile_to_world`, …) re-exported so nothing breaks.
- `game_shared` keeps **zero dependencies** for as long as it can. Adding one needs a reason written in the PR.
  `serde` will arrive with Phase 5 for the protocol and is the expected first one.
- `game_shared` must never depend on `bevy`. CI enforces this implicitly once the server crate exists; until then
  QC checks `Cargo.toml`.

## Client structure (Bevy)

- One `Plugin` per file above. `main.rs` only adds plugins.
- `WorldState` is a Bevy `Resource`. Systems never change it directly; they send an `Action` message and one
  system, `apply_actions`, calls `game_shared::apply` and forwards the resulting `Event`s as Bevy messages. View
  systems (`world_view`, `hud`) read those. This keeps the client honest: it already behaves as if a server were
  answering.
- Simulation (`physics::step`, `apply_actions`) runs in `FixedUpdate` at 60 Hz. Rendering, camera and interpolation
  run in `Update`.
- Entities are views. A tile sprite or a player sprite holds no gameplay truth; deleting every sprite and
  rebuilding them from `WorldState` must give the same picture.

### Asset path

`crate::asset_dir()` in `main.rs` picks the first existing of `../../assets` (`cargo run`), `assets` (next to the
binary) or `../Resources/assets` (macOS `.app` bundle). Code that reads asset files from disk directly must use it;
there is no `ASSET_DIR` constant any more.

### Bevy version warning

The workspace pins **Bevy 0.20**. Its API moves between releases (events/messages, sprite and atlas types, required
components, UI). Before writing a system, check the 0.20 docs or the examples in the matching Bevy git tag, and
follow the patterns already in `main.rs` (`Camera2d`, `Sprite::from_color`, `time.delta_secs()`). Code copied from
older tutorials will not compile.

### Camera

- Follows the player with exponential smoothing: `pos += (target - pos) * (1 - exp(-CAMERA_STIFFNESS * dt))`
  (`CAMERA_STIFFNESS = 10`, Tuning). This form is frame-rate independent, unlike a plain `lerp(pos, target, 0.1)`.
- Clamped so the view never shows outside the world rectangle. If the window is larger than the world on an axis,
  center on that axis.
- Pixel art: use nearest-neighbour sampling (`ImagePlugin::default_nearest()`), and round the final camera position
  to whole pixels at the current zoom to avoid tile seams and shimmering.

## Code rules (QC checks these)

| Rule | Why |
|---|---|
| `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass | Same as CI |
| No gameplay rule in the client crate | It would have to be rewritten for the server, and would be exploitable |
| No literal tuning numbers in systems | Named constants in `game_shared` only |
| No id comparisons outside the named constants | Behaviour comes from the item table |
| No `unwrap`/`expect`/unchecked indexing on action-derived values in `game_shared` | Player input must not crash a server |
| Every milestone adds the tests listed in its doc | "Tests that must exist" sections are the definition of done |
| Deliberate shortcuts get a `// ponytail:` comment naming the ceiling and the upgrade path | So a later reader knows it was a choice |
| No new dependency without a stated reason | Compile time and supply-chain surface |
