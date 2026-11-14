# UX · VFX for punching, farming and drops

Spec for the implementer and aa. Goal: every action has a visible answer, without a noisy screen. All effects are
client-only, driven by the `Event`s the client already receives (`TileDamaged`, `TileBroken`, `TreePlanted`,
`TreeSpliced`, `ItemDropped`, `DropTaken`). No protocol change.

## Budget and the "Reduce effects" toggle

- **Cap: 200 live particles** in total. A new burst that would exceed it spawns fewer particles; nothing queues.
- Particles are code-drawn 2×2 px squares (no art). Sprite effects play once and despawn.
- **Settings > Display > `Reduce effects`** (off by default; uses `ui/settings/toggle_*`): particle counts ×0.25,
  no screen shake, no bob, sway or glow on drops (the toss arc stays, it shows where the item went). Pickup fly, floating text and the splice result icon stay (they carry
  information).
- **`Screen shake`** toggle next to it, **off by default**. When on: 2 px, 0.1 s, on block break only.

## (a) Punch and break

| Event | Effect |
|---|---|
| Each punch (`TileDamaged`) | 4 particles from the hit point, sprayed away from the player, in the block's colour, 0.25 s life with gravity |
| Break (`TileBroken`) | 12 particles in the block's colour plus `fx/block_break_dust.png` tinted to the block colour, centred on the tile |

**Reach-out arm** (owner request, revised 2026-11-14): on every punch the front arm jabs toward the target and snaps
back.

| Part | Rule |
|---|---|
| Held item | Lives on the **back arm** (`ArmBack`), at its hand behind the body, all the time, swings included, resting lifted at −45° so it doesn't look dragged. The reaching front arm is always the bare fist; it never turns a tool to strike |
| Direction | From the current shoulder toward the target tile, any angle (up, down, diagonal), re-aimed every frame |
| Length | At most **1 tile beyond the arm's rest length**: a jab, not a pole to the tile. Half that when the tile is out of reach; ×0.6 for a place |
| Shape | Tapered: outline 4 px at the shoulder widening to 7 px just before the fist, skin 2 → 5 px inside it, in the arm's own outline colour and skin ramp. The fist sprite at the tip is not stretched. An up-punch steeper than ~60° from horizontal is drawn just behind the head, so the face stays visible |
| Timing | Out 0.08 s (ease-out), back 0.10 s (ease-in); the angle eases out from rest and back to rest (no snap). A new punch during the return restarts from the current length |
| Contact | `ArmContact` at full extension spawns the hit particles (not the click) |
| Facing | The body faces the target while the arm is out; walking doesn't flip it mid-swing |
| Placing | The same jab at 0.6 length and 0.06 s / 0.08 s; the bare fist reaches (the held block stays on the back arm) |
| Refused punch | The arm still jabs (the player did swing); the refusal feedback (U2/claims) plays as usual |
| Remote players | Everyone sees everyone's swings, driven by `Event::Swing { player, x, y, place }` from the server; the local player animates on input and ignores its own `Swing` |
| Reduce effects | Kept: it's core feedback, not decoration |

Block colour: the average opaque colour of the block's sprite, computed once when the sprite loads and cached per item.
Seeds already carry an `rgb`; use it for saplings.

## (b) Planting and splicing

| Event | Effect |
|---|---|
| `TreePlanted` | `fx/soil_puff.png` at the tile's base, and the sapling sprite pops: scale 0.6 → 1.1 → 1.0 over 0.2 s |
| `TreeSpliced` | **The most noticeable farming effect.** `fx/splice_swirl.png` over the tree, the same sapling pop, the **result item's icon popping up above the sapling** (scale 0 → 1.2 → 1.0 in 0.2 s, held, faded out by 1 s: the A + B → C moment), and `sfx/splice.ogg` (0.8 s, exists) |
| Harvest | Unchanged: the drops popping out are the effect |

## (c) Drops and pickup

- **Toss** (`ItemDropped` from the local or any visible player): the item leaves the player's hand in a short arc
  (0.25 s, peak 12 px, toward the side they face) and lands on the drop's position. No particles.
- **Wobble:** every drop bobs 3 px up and down on a 1.2 s sine and sways ±4° on a 1.8 s sine, both phases from its
  drop id, so a pile never moves in sync. Reduce effects: a still item, no bob or sway.
- **Glow:** only seed drops (the small, valuable ones) get `fx/drop_glow.png` under them, alpha pulsing 0.4–0.8.
- **Pickup** (`DropTaken` by the local player): the item icon flies from the drop to the player's chest in 0.2 s
  (ease-in) and ends with one `fx/ready_sparkle.png`. Floating text above the head: `+1 Dirt Seed` in the pixel font at its full 16 px, white with a 1 px
  dark shadow so it reads on sky and caves, rising 16 px and fading over 0.8 s. Pickups of the same item within 0.5 s merge into one text (`+3 Dirt`).
- Other players' pickups: the fly to that player only, no text.

## (d) Farmable at a glance

- **Seed cells** (inventory, hotbar, store, trade) show the parent item's icon at 12 px in the bottom-left corner.
  This tells the 20+ look-alike seeds apart without hovering. Uses `items::seed_parent`, no new art.
- **Tooltip line** (item-tooltip.md, line 3): seeds read `Grows: <Item> · <t>`. Items with a seed form
  (`items::seed_of` is `Some`) add `Farmable` after their line 3 text (e.g. `Block · 3 punches · Farmable`).
  No badge on non-seed cells: most blocks are farmable, so a badge on every cell would be noise.

## Art (all done by aa; 12 fps, no loop, centre pivot, json sidecars; plus `audio/sfx/splice.ogg`, 0.8 s)

| File | Frames × size | Look |
|---|---|---|
| `fx/block_break_dust.png` | 6 × 32×32 | Neutral light-grey dust and chips bursting out, so it can be tinted to any block colour |
| `fx/soil_puff.png` | 5 × 32×32 | A small brown soil puff hugging the ground, low and wide |
| `fx/splice_swirl.png` | 8 × 32×32 | A green and violet spiral of leaves/sparks that tightens into the tree; clearly not the soil puff |
| `fx/drop_glow.png` | 1 × 16×16 | A soft warm ring/glow, transparent centre, drawn under a drop |

## Do NOT build

- No particles on walking, jumping or landing; no trails; no hit-stop.
- No damage numbers on blocks (the crack sprites already show progress).
- No effects for things other players do far off screen (only what the camera shows).
