# 17 · Growtopia Reference (mechanics only)

Research pass, 2026-11-14. Growtopia is the best-known game in our genre and the "reference" our other docs mean.
This doc records **how its systems work** so we can borrow structures, not content. Per the
[originality rule](README.md#originality-rule), none of its item names, art, recipes or world names go into our game.
Names below are cited only to identify the mechanic.

Labels in this doc: **Verified** (read in the cited source), **Inferred** (worked out from verified numbers, the
reasoning is given), **Uncertain** (single community source or not confirmed).

Source caveat: the Fandom wiki's item infoboxes were frozen in February 2025 and its sprites removed at the
publisher's request, so per-item numbers there may be stale. Numbers were cross-checked on growtopiawiki.com where
possible.

## 1. Player stats

Source: <https://growtopia.fandom.com/wiki/Mods>, <https://growtopia.fandom.com/wiki/XP>,
<https://growtopia.fandom.com/wiki/Fist>, <https://growtopiawiki.com/w/Fist>,
level curve from community server and calculator code:
<https://github.com/StileDevs/GrowServer/blob/main/src/game/player/index.ts>,
<https://github.com/xSkriptx/Growtools/blob/main/src/pages/tools/LevelCalculator.tsx>.

### What a player actually has

Growtopia has **no visible stat sheet** (no strength, agility, HP bar in normal play). A player is:

| Stat | Value / behaviour | Label |
|---|---|---|
| Gems | The one soft currency, a number on the account. Earned from breaking blocks and harvesting trees (about +1 gem per 4 rarity), bought with real money, spent in the store | Verified |
| Level and XP | Level 1 to **125**. Cosmetic rewards at 50, an "epic quest" step at 100, blue name at 125 | Verified |
| Punch | Base fist: one punch per **0.2 s**, reach **3 tiles** (wiki wording; see note), block damage restores after a per-block idle time (4–8 s) | Verified |
| Movement | Fixed base walk speed and jump. No per-player numbers are published | Uncertain (no source gives the base values) |
| Health | No HP bar. "Deadly" blocks (spikes) kill on touch and respawn you at the last checkpoint or the main door; "pain" blocks (lava, acid) knock you back and kill after repeated contact. Mods such as *Damage Reduction*, *Healing*, *Fireproof* imply a hidden health value | Uncertain (behaviour described, numbers unknown) |
| Inventory | Slots and a 200-per-item cap, already in [08](08-inventory-and-trading.md) | Verified earlier |

Reach note: the wiki says "3 tiles by default". Our [05](05-punch-and-build.md) reads the same reference as "2 tiles
beyond your own tile", i.e. a 3-tile span counting your own. Both readings agree on the in-game feel; keep ours.

### Level curve

```
xp_to_next(L) = 50 × (L² + 2)                     (community-derived, Uncertain but consistent across 3+ sources)
total_xp(L)   = 50 × ((L−1)·L·(2L−1)/6 + 2·(L−1))
```

| Level | XP to the next level | Total XP to reach it |
|---|---|---|
| 2 | 300 | 150 |
| 5 | 1,350 | 1,900 |
| 10 | 5,100 | 15,150 |
| 25 | 31,350 | 247,400 |
| 50 | 125,100 | 2,026,150 |
| 100 | 500,100 | 16,427,400 |
| 125 (max) | — | 32,174,900 |

### Where XP comes from (Verified, XP page)

| Activity | XP |
|---|---|
| Breaking a block or harvesting a tree | `1 + floor(rarity / 5)` |
| Using any consumable, harvesting a provider, putting out a fire | 1 |
| Side activities (surgery 150, forging 100, chem synthesis 100, fishing 1 per 10 lb, …) | 1–1,000 each |
| Collecting gems | 0 (removed in a 2016 patch) |

XP is multiplied by **XP Buff** mods, each scoped to one activity (fishing, cooking, surgery, …).

### Playmods: how clothing and consumables change a player

A **mod** ("playmod") is a named ability attached to a worn item, a consumable or an event. Worn items apply it
while equipped; consumables apply it for a fixed time. Mods from several items are combined, but each mod has its own
stacking rule.

| Mod | Effect | Stacking | Items carrying it (wiki category count) | Label |
|---|---|---|---|---|
| Double Jump | One extra jump in mid-air | Binary | ~393 | Verified |
| High Jump | Higher jump | Binary (assumed) | ~78 | Verified (magnitude Uncertain) |
| Slow Fall | Glide down slowly while holding jump | Binary | ~72 | Verified |
| Speedy | Faster walking on land | Binary (assumed) | ~308 | Verified (magnitude Uncertain) |
| Speedy in Water / Float on Water | Faster swimming / stand on water | — | ~25 / few | Verified |
| Wall Climbing | Cling to and climb walls | — | ~9 | Verified |
| Punch Damage (old name "Enhanced Digging") | Fewer hits to break blocks | **Does not stack** | ~314 | Verified |
| Punch Range | Punch 1 tile further | Stacks to **+2** | ~325 | Verified |
| Build Range | Place 1 tile further | Like punch range (assumed) | ~90 | Verified |
| Punch Power | Knock other players further | Stacks almost without limit | ~64 | Verified |
| Punch Pull | Punching pulls players in instead of pushing | — | ~10 | Verified |
| Damage Reduction (as a side effect) | Weaker punch: more hits to break (e.g. a range-boosting visor trades damage for reach) | — | — | Verified |
| Extra Gems / Extra Blocks / XP Buff | More gems, blocks or XP from farming | — | few, mostly the rare "artifact" slot | Verified |
| Light Source / Night Vision | See in dark worlds | — | ~70 | Verified |
| Fireproof, Ghost Immunity, Knockback Reduction, Healing | Hazard resistances | — | — | Verified |

Timed (consumable) mods worth noting: a 5 s speed burst from a drink or pepper, a few seconds of floating upward,
6 s of slow-and-low-jump from a hazard, 10-minute social penalties (muted chat, cannot leave the world), and a timed
"Lucky" mod that gives a chance of 5× gems from breaking.

## 2. Pickaxes and punching tools

Source: <https://growtopia.fandom.com/wiki/Fist>, <https://growtopia.fandom.com/wiki/Category:Punch_Damage_Mod_Items>,
item pages (e.g. <https://growtopia.fandom.com/wiki/Pickaxe>, <https://growtopia.fandom.com/wiki/Cyclopean_Visor>,
<https://growtopia.fandom.com/wiki/Rock_Hammer>), hardness values from <https://growtopiawiki.com/w/Rock> and
<https://growtopiawiki.com/w/Steel_Block>, item format from
<https://github.com/RebillionXX/GTServer/blob/master/src/database/item/item_database.cpp>.

### How "hits to break" works

1. Every block has a **hardness** in hits and a **restore time** (seconds of no punching before damage resets,
   typically 4–8 s, set per item). Verified.
2. In the item file the hardness is stored as **hits × 6**; servers divide by 6 on load. Verified (server source).
3. Each item page shows two hardness numbers: plain fist and with the Punch Damage mod.

| Block | Rarity | Fist hits | With Punch Damage | Restore |
|---|---|---|---|---|
| Dirt | 1 | 3 | 3 | 8 s |
| Rock | 1 | 10 | 8 | 5 s |
| Steel block | 35 | 20 | 15 | 8 s |
| (the fist's own data) | — | 4 | 3 | 8 s |

**Inferred:** a fist punch deals 6 damage units and a Punch Damage punch deals 8 (+33 %), with
`hits = ceil(hardness×6 / damage)`: 3 → ceil(18/8) = 3, 10 → ceil(60/8) = 8, 20 → ceil(120/8) = 15. That is why the
mod does not stack: it is one fixed damage value, not a multiplier.

### The tools

There is **no tier ladder** of pickaxes. Almost every breaking tool grants the same single Punch Damage mod; they
differ in how they are obtained, how they look, and in side effects.

| Tool (reference name) | Break effect | Other effects | How obtained |
|---|---|---|---|
| Basic pickaxe | Punch Damage | — | Splice two common seeds, or a random clothes pack (500 gems) |
| Golden / emerald pickaxe | Punch Damage, same as the basic one | Status symbol only, says so in its own description | Store for event tokens / crafted from 100 shards |
| Death ray | Punch Damage | Different punch visual | Bonus in a 5,000-gem store pack |
| Diamond pick (event) | Punch Damage | — | Event crafting |
| Rocket warhammer | Punch Damage | Visual | Challenge reward |
| Climbing picks | Punch Damage | Climb special climbing walls | Crafted |
| Dragon summoning hand | Punch Damage + Punch Power | A pet dragon replaces the punch animation with fire | Store, 50,000 gems |
| "Infinity" fist | Punch Damage + Punch Range | Customisable via wrench | Paid event |
| Range visor (face item) | **Weaker** punch (more hits) | Punch Range +1 | Splice or cheap clothes pack |
| Fossil hammer / chisel | **Weaker** punch | Shorter range, punch pull; required to extract fossils without smashing them | Paleontology store kit |

About 30 hand items carry Punch Damage; the mod exists on every slot (hats, shirts, back items…), ~314 items in all.
The fist punches at 0.2 s intervals; no item found changes the punch rate (Uncertain).

## 3. Wings and back items

Source: <https://growtopia.fandom.com/wiki/Category:Wings>, <https://growtopia.fandom.com/wiki/Mods/Double_Jump>,
<https://growtopia.fandom.com/wiki/Mods/Slow_Fall>, item pages for the rows below.

The back slot is the movement slot. **47 wings** and ~14 capes are listed; nearly all wings grant exactly one mod,
Double Jump. Rarity and prestige come from scarcity and how they are obtained, not from better physics. One wing
item's own description says it does not jump higher than normal wings.

| Item (reference name) | Movement effect | How obtained | What sets it apart |
|---|---|---|---|
| Fairy wings (the first wings) | Double jump | Splice two mid seeds | Cheap entry wings |
| Angel / devil wings | Double jump | Seasonal event drop | Event-limited |
| Rainbow wings | Double jump | Splice two high-rarity gem blocks | Farmable |
| Golden angel wings | Double jump | Craft: angel wings + 200 event items + crystal | Upgrade path from a lower wing |
| Diamond wings | Double jump | Carnival minigame prize | Minigame reward |
| Bubble wings | Double jump + bubble visual | Rare roll from a seasonal egg (90–99 %) | Luck |
| Phoenix / draconic wings | Double jump | Fixed supply (500 / 1,200 ever made) | Scarcity |
| Pegasus wings | Double jump + feather burst | Store | Visual |
| Steampunk glider wings | Double jump **+ slow fall** | Craft from 4 wings + parts | Two mods |
| Legendary quest wings | Double jump **+ slow fall** | 20-step quest (deliver 100,000 gems, 600 cheap wings, break 5,000 blocks, …) | Long-term goal, untradeable |
| Jetpack (back) | **High jump**, no double jump | Splice lava + steel | Alternative to wings |
| Parasol (hand) | **Slow fall** while holding jump | Splice, or a store pack | Glide without wings |
| Speed shoes (feet) | **Speedy** | Splice, or racing store pack | Speed lives on feet and mounts |
| Mounts (horse, dinosaur, hoverboard, bike) | **Speedy** | Store only | Speed as a premium cosmetic |

Takeaway from the table: movement is split by **slot**, not by tier. Back = air control (double jump, glide, high
jump), feet and mounts = ground speed, hand = punch.

## 4. Item list

Source: <https://growtopia.fandom.com/wiki/Items>, <https://growtopia.fandom.com/wiki/Rarity>,
<https://growtopia.fandom.com/wiki/Gems>, item-type enum from
<https://github.com/RebillionXX/GTServer/blob/master/src/database/item/item_type.h>, item fields from
<https://github.com/eikarna/GoDat/blob/main/Components/Enums/Enums.go>.

### Size

| Measure | Count | Label |
|---|---|---|
| Wiki pages in `Category:Item` | ~7,460 | Verified (2026-11-14) |
| Splicable / unsplicable | ~1,140 / ~5,770 | Verified (wiki categories) |
| Ids in the item file | About twice the item count: every item has a seed at `item id + 1` | Inferred (exact current count not found) |
| Distinct item behaviours ("action types") | ~140 | Verified (server enum) |

### Properties every item has (item file fields)

| Field | Meaning |
|---|---|
| id, name | Seeds are a separate id next to their item |
| action type | Behaviour class (block, background, seed, clothes, lock, door, consumable, …), ~140 values |
| clothing type | Slot, for clothes only (list below) |
| collision type | Solid, none, platform-like, etc. |
| break hits | Hardness × 6 |
| restore time | Seconds before punch damage resets |
| rarity | Sum of parent rarities for spliced items; drives gems and grow time (999 for special unrated items) |
| grow time | Tree grow time; matches `rarity³ + 30·rarity` s (Dirt 1 → 31 s, Steel 35 → 12 h 12 m 5 s), same as our [06](06-farming-and-splicing.md) |
| max amount | Stack cap (200 for normal items) |
| drop chance, seed colours, tree base/leaf sprites | Farming and seed looks |
| flags | Untradeable, unsplicable, permanent, etc. |
| editable type, texture, hit sound, pet fields, punch option | Presentation and special behaviour |

### Clothing slots (10, from the item file)

hair, shirt, pants, feet, face, hand, back, hat ("mask"), chest ("necklace"), and an **artifact** slot for rare
stat items (extra gems, extra blocks, XP buffs). We have 8; we lack chest and artifact.

### Categories with representative examples

Counts are approximate where given; wiki category counts undercount because many pages are tagged by template.

| Category | Examples (reference names, mechanics only) | Notes |
|---|---|---|
| Foreground blocks | dirt, rock, sand, grass, wood block, glass pane, brick, steel block, gem-colour blocks, crystal block (rarity 100) | Solid; most splicable. Hardness 3–20+ |
| Backgrounds | cave background, wood wall, wallpaper, window, art wall | Never collide; must be broken after the block in front |
| Non-solid decor | flowers, signs, lanterns, chairs | Walk-through |
| Platforms | wooden platform, cave platform | Solid from above |
| Hazards | death spikes, lava, acid, evil eye (toggleable) | Deadly (kill) vs pain (knockback) |
| Bouncy / slippery / pushing | mushroom trampoline, pinball bumper, ice, wind blocks | Physics tiles |
| Doors and portals | door, entrance, portal to another world, checkpoint, main ("white") door | Travel and respawn |
| Seeds | one per item, `id + 1` | Splicing two seeds makes a new item |
| Locks | small (10 tiles), big (48), huge (200), world lock (2,000 gems), builder's lock, 100 world locks combine into a diamond lock | Land ownership and the de-facto trade currency |
| Clothes | 10 slots above; hundreds per slot | Cosmetic, some carry mods |
| Consumables | foods and potions with timed mods, XP potion (10,000 XP), snowball (freeze), duct tape (garble chat) | One-shot, many are social |
| Machines | vending machine, item magnet/planter, weather machines (~15), chemical combiner, oven, forge, sewing machine | Automation and crafting stations |
| Providers | chicken, cow, science station | Produce items on a timer |
| Storage and social | chests, mailbox, bulletin board, donation box, mannequin, display block | World furniture with state |
| Treasure / random | mystery block (10–50 gems), dice, event piñatas, magic egg | Loot tables |
| Pets | ~400 | Follow the player, some add mods |
| Artifacts | rare "ancestral" items | Strongest stat boosts |

### Full lists

- Wiki item hub and per-type pages: <https://growtopia.fandom.com/wiki/Items>, <https://growtopia.fandom.com/wiki/Clothes>
- Per-mod item lists: <https://growtopia.fandom.com/wiki/Mods>
- Wiki API, category members (works where the HTML returns 402):
  `https://growtopia.fandom.com/api.php?action=query&list=categorymembers&cmtitle=Category:Wings&cmlimit=500&format=json`
- items.dat parsers: <https://github.com/eikarna/GoDat> (Go), <https://github.com/houzeyhoo/gt-itemsdat-exporter>
  (Python, CSV/JSON export), npm `grow-items`. These need a copy of the client's `items.dat`; we do not store one.

## 5. Takeaways for Growsociety

Compared with what we already have: [03](03-items-and-tiles.md) (8 slots, cosmetic clothing, wings deferred),
[05](05-punch-and-build.md) (reach 2, damage reset 5 s global), [08](08-inventory-and-trading.md),
[14](14-levels.md) (`100 × L` curve, 775,000 XP to 125).

| # | Mechanic | What we have | Suggestion (original form) |
|---|---|---|---|
| T1 | **Named modifiers on items** | Only pickaxe `power` | A small `Mod` enum on `ItemDef` (e.g. `ExtraJump`, `Glide`, `JumpBoost`, `RunBoost`, `ReachUp`, `DigBoost`, `GemFinder`) with one rule per mod: a mod is on or off, except reach which adds up to a cap. Avoids per-item stat columns and keeps balance auditable. Our names, not theirs |
| T2 | **Movement by slot** | Wings cosmetic, double jump "later" | Back slot = air (extra jump, glide), shoes = ground speed. Start with one mod: a single mid-air jump for every back item, applied inside `step` (the physics doc already flags this). Glide as a second, rarer mod. Do not make better wings jump higher; prestige comes from how they are earned |
| T3 | **Dig speed** | Pickaxe tiers 1–4 (power 1–4, up to 3× fewer punches) | The reference has one flat +33 % with no tiers, so tools feel like a style choice, not a grind ladder. Our 4-tier ladder is a deliberate difference; consider capping the top tier near 2× so high-hardness blocks still need claims and time. Store damage in sixths (or another sub-unit) so small boosts are expressible |
| T4 | **Trade-off items** | None | Items that give reach but weaken digging (or vice versa) make choices interesting. One or two such items in our set, with our own names |
| T5 | **Per-block restore time** | One global 5 s | Reference uses 4–8 s per block. Keep global until a block needs it; the field is cheap to add later |
| T6 | **Level curve** | `100 × L`, 775,000 XP to 125 | Reference is quadratic per level (`50(L²+2)`), ~32 M XP to 125, and gives only `1 + rarity/5` XP per break. Ours is ~40× faster and gives more per break. Fine for now (nothing is gated by level); revisit before adding level gates or level-based rewards |
| T7 | **Milestone rewards** | Name colour only | Cosmetic unlocks at fixed levels (e.g. a pet or title at 50) give the curve a purpose without gating play |
| T8 | **Activity-scoped XP boosts** | None | "+XP from farming" style items or timed buffs; cheap once T1 exists |
| T9 | **Timed consumables** | `Resource` kind reserved | Short buffs (5 s run boost, brief float) from crops we already grow give berries/wheat a use. Social consumables (freeze, garble chat) need moderation thought first ([13](13-commands-and-moderation.md)) |
| T10 | **Chest and artifact slots** | 8 slots | A 9th "charm" slot for rare passive items (gem finder, XP boost) is the cleanest place for economic mods, so clothing stays cosmetic |
| T11 | **Long quests for top items** | Wings at 10,000 gems in the store | A multi-step quest (deliver lower wings, break N blocks, plant N rarity) as the source of the best back item turns it into a long goal and a sink for lower items |
| T12 | **Seed id = item id + offset** | `1000 + id` | Same idea, already done. No change |

Do not adopt: their item names, recipes, the lock tile counts beyond what [07](07-economy-and-locks.md) already
labels as Reference, fixed-supply "only N exist" items (needs a live-ops promise we cannot keep yet), and paid
random packs (deferred in [03](03-items-and-tiles.md) until odds are published).
