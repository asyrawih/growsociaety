# 14 · Levels and XP

First pass, owner request 2026-11-13 (relayed by the asset session). Keep it minimal: XP from the three core verbs, a
level curve, the level visible to others, a level-up toast and sound. Nothing is gated by level yet.

## Knowledge

### Where XP comes from (all Tuning)

| Action | XP | Why |
|---|---|---|
| Breaking a block or background | `max(1, rarity)` | Rarer blocks are worth more; base blocks still give 1 |
| Harvesting a grown tree | `XP_HARVEST_PER_RARITY (2) × rarity` | Farming is the core loop; waiting deserves more |
| Planting a seed (not splicing) | `XP_PLANT = 1` | Small nudge to farm |
| Splicing two seeds | `XP_SPLICE = 2` | Rewards discovery a little |
| Destroying a sapling, breaking a claim, admin commands, buying, trading | 0 | Not skill or effort; claims would be an infinite XP source |

XP is credited to the player whose action caused it, through `apply` like gems.

### Level curve

```
xp_to_next(level) = XP_CURVE_BASE × level        with XP_CURVE_BASE = 100
```

| Level | XP to the next level | Total XP to reach it |
|---|---|---|
| 1 | 100 | 0 |
| 2 | 200 | 100 |
| 5 | 500 | 1,000 |
| 10 | 1,000 | 4,500 |
| 25 | 2,500 | 30,000 |
| 50 | 5,000 | 122,500 |
| 125 (max) | — | 775,000 |

Total XP grows with the square of the level (`50 × L × (L − 1)`), so early levels come in minutes and late levels take
real farming. `MAX_LEVEL = 125` (Reference: the genre's cap). At the cap, XP stops accumulating.

### What the level does

| Effect | Where |
|---|---|
| Shown above the player's head as the name colour | The rig's `name_color(level)` hook (coding session, `player.rs`) |
| Level-up toast "Level N!" and `sfx/level_up.ogg` | Client HUD and audio |
| Level number and XP bar in the HUD | Client HUD, next to the gem counter |
| Gating (e.g. trading or world claims from level X) | **Not in this pass.** Add only when the owner asks |

## Technical

```rust
// Player (rules.rs) gains:
pub level: u16,   // 1..=MAX_LEVEL, starts at 1
pub xp: u32,      // progress within the current level, 0..xp_to_next(level)

// game_shared (e.g. levels.rs):
pub const MAX_LEVEL: u16 = 125;
pub const XP_CURVE_BASE: u32 = 100;
pub fn xp_to_next(level: u16) -> u32;          // 0 at MAX_LEVEL
pub fn add_xp(level: &mut u16, xp: &mut u32, gained: u32) -> u16;  // returns levels gained; saturating, carries over

Event::XpChanged { player: PlayerId, level: u16, xp: u32 }   // private: only the player
Event::LevelUp { player: PlayerId, level: u16 }               // public: everyone in the world (name colour)
```

- One action can gain several levels at once (`add_xp` loops). Emit `XpChanged` once per action with the final values
  and one `LevelUp` with the final level.
- `XpChanged` is private (like inventory and gems); `LevelUp` is public. The join snapshot carries every player's
  level (not XP), and `PlayerJoined` carries the newcomer's level, so name colours are right on arrival.
- New `cfg_attr(feature = "serde", …)` derives on anything new, and the mirror (`game_shared::mirror`) applies both
  events.
- Persistence: `level` and `xp` are saved with the player (the coding session's persistence work).

### Tests that must exist

- `xp_to_next` matches the table; `add_xp` carries over across several levels, stops at `MAX_LEVEL`, never overflows.
- Breaking stone (rarity 1) gives 1 XP; breaking a rarity-3 block gives 3; harvesting a rarity-2 tree gives 4;
  planting gives 1; splicing gives 2; destroying a sapling and breaking a claim give 0.
- A level-up emits exactly one `LevelUp` with the final level; `XpChanged` carries the post-action values.
- A rejected action changes neither XP nor level (extend the existing reject test).
- The mirror reproduces level and XP from the events.
