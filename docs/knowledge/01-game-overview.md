# 01 · Game Overview

## What the game is

A 2D side-view multiplayer sandbox. Every world is a small, fixed-size grid of tiles that players break, build,
farm and own. Worlds are entered by name; a name nobody has used yet creates a fresh world. Progress is not levels
or combat, it is **what you own and what you can grow**.

## Design pillars

1. **Small worlds, many of them.** A world is 100 tiles wide: you can see a meaningful part of it at once, and the
   whole thing fits in one network message. Scale comes from the number of worlds, not their size.
2. **Everything is an item.** Blocks, backgrounds, seeds, tools and claims all live in one item table and one
   inventory. There is no separate "building mode".
3. **Farming is the economy.** Blocks come from trees, trees come from seeds, new items come from combining seeds.
   Time (grow time) is the main cost, and it rises steeply with rarity.
4. **Ownership is physical.** You own tiles by placing a claim block on them. Protection is visible in the world,
   not hidden in a menu.
5. **The server decides.** The client asks, the server answers. No gameplay result is ever trusted from a client.

## Core loop

```
enter world ──> punch blocks ──> get blocks + seeds + gems
                     ^                     │
                     │                     v
              harvest trees <── plant seeds (wait: grow time)
                     │                     │
                     v                     v
              build / trade          splice two seeds ──> new, rarer item
                     │
                     v
        buy claims with gems ──> own tiles / a world ──> safe place to farm more
```

One cycle at the bottom of the tree (rarity 1–2) takes about a minute. Higher rarities take hours or days, which is
why players need protected land and why they trade.

## Player-facing systems, in dependency order

| System | Depends on | Doc |
|---|---|---|
| World + tiles | — | [02](02-world.md), [03](03-items-and-tiles.md) |
| Movement | World collision | [04](04-player-physics.md) |
| Punch / place | World, items, movement (range) | [05](05-punch-and-build.md) |
| Inventory | Items | [08](08-inventory-and-trading.md) |
| Seeds, trees, splicing | Punch/place, inventory, time | [06](06-farming-and-splicing.md) |
| Gems | Punch (drops) | [07](07-economy-and-locks.md) |
| Claims / ownership | Punch/place, gems | [07](07-economy-and-locks.md) |
| Multiplayer | All of the above running through Actions | [10](10-networking-and-persistence.md) |
| Trading | Inventory, multiplayer | [08](08-inventory-and-trading.md) |

This order is the build order in [11](11-implementation-plan.md).

## What we deliberately do not build yet

| Not yet | Why | Add when |
|---|---|---|
| Combat, health, enemies | Not part of the core loop | The owner asks for it |
| Clothing / cosmetics | Needs trading and persistence first | After Phase 7 |
| Store with real money | Needs accounts, persistence, legal review | After Phase 7 |
| World variants (tiny, wide) | One size proves the loop | A design reason appears; sizes are already constants |
| Chat, friends, moderation tools | Needs the server | Phase 5+ |

## Current state of the repo (2026-11-13)

- `crates/game_shared`: world size constants and tile↔world coordinate conversion, with one test.
- `client/game_client`: a Bevy app that spins a green square. No world, no player.
- `assets/`: terrain autotile atlas, 58 block sprites, 16 objects, player sheets. Art is ahead of code.
- No server, no persistence, no commits yet.
