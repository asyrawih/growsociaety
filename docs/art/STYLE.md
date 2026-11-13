# Master Style Reference: PixelLab

Reference board: [`style-board.png`](style-board.png). Upload it to PixelLab as the **style reference** for every generation.
It is a style sample only. Never use it as an init image or a layout.

---

## 1. Style lock (paste into every prompt)

```
painterly pixel art, warm natural colors, soft atmospheric lighting, light from top-left,
organic terrain textures, detailed foliage, readable silhouette, side-view orthographic,
cozy fantasy village, crisp pixels, no anti-aliasing blur, consistent with style reference
```

## 2. Negative prompt

```
reference board layout, UI panel, labels, text, grid, multiple assets on one canvas,
isometric, top-down, 3/4 view, perspective camera, photorealistic, 3D render, vector art,
smooth gradient, blurry, noisy dithering, neon oversaturated colors, harsh black shadows,
thick black outline, watermark, cropped
```

## 3. Rules when generating

1. Keep the **painterly pixel art** look: soft clusters of color, not flat cel shading.
2. Keep the **palette and lighting**: light comes from the top left, shadows are warm brown/purple, never pure black.
3. Keep the **side-view orthographic** camera, like a platformer. No isometric, no top-down.
4. **Don't copy the board layout.** Generate one asset per canvas, with a transparent background.
5. **Generate each asset separately**, every time with the same style reference, style lock and negative prompt.

## 4. Recommended PixelLab settings

| Setting | Value |
|---|---|
| View | Side |
| Outline | Selective outline (dark, colored by the base color, not pure black) |
| Shading | Detailed |
| Detail | Highly detailed for props/nature, Medium for characters (readable at small sizes) |
| Background | Transparent / no background (except parallax) |
| Style strength | High |

## 5. Scale (matches `game_shared::TILE_SIZE = 32`)

| Asset | Canvas | Notes |
|---|---|---|
| Tile | 32×32 | Must tile seamlessly left/right and up/down |
| Character | 48×48 | Body ~1.5 tiles tall, feet at the bottom edge |
| Small object (chest, sign, crate) | 32×32 | Sits on the floor line |
| Large object (door, forge, bed) | 64×64 | |
| Tree | 64×96 or 96×128 | Roots on the floor line |
| Plant / flower / growth stage | 32×32 | Same base (soil mound) at every stage |
| Parallax layer | 480×270 (scale up ×4) | Wraps horizontally |

## 6. Palette (sampled from the board)

| | | | |
|---|---|---|---|
| `#93AA36` grass light | `#467143` leaf dark | `#757147` moss | `#6B4937` soil dark |
| `#976040` wood | `#CA955F` sand / wood light | `#847B84` stone | `#C9C1C7` stone light |
| `#93A5B9` mist / far | `#3E759D` water / sky deep | `#8BCEF9` sky / ice | `#5D5F77` shadow cool |
| `#AA4447` scarf / roof red | `#FBA451` lantern / fire | `#9894B0` metal | `#7A61C2` magic / crystal |

If PixelLab supports palette input, upload these 16 colors. Otherwise use the board as the color reference.

---

## 7. Prompt templates

Format: `[subject] + [details] + [style lock]`, with the negative prompt from section 2.

### Character (base)
```
young adventurer boy, messy brown hair, aviator goggles on head, red scarf, brown leather jacket,
dark pants, brown boots, chibi proportions, facing right, [style lock]
```
Animations: use the same character and canvas, one animation per generation (facing right; mirror it in code for left).

| Animation | Frames | Add to prompt |
|---|---|---|
| Idle | 4 | `idle breathing, subtle bob` |
| Walk | 6 | `walk cycle` |
| Jump | 2 | `jumping up, knees tucked` |
| Fall | 2 | `falling, arms raised` |
| Punch | 4 | `punching forward with fist` |
| Break | 4 | `swinging pickaxe forward, hitting block` |

### Tiles (32×32, seamless)
```
[grass-topped dirt | dirt | cobblestone | snowy stone | sand | wooden planks | stone bricks |
 mossy dirt | vine-covered dark soil | mossy stone | wooden crate | log end rings | wooden wall |
 riveted metal plate | glass | water | lava | purple crystal ore] block tile, seamless tileable,
flat front face, side view, [style lock]
```
Use the side-scroller tileset tool if it's available. For grass, make a top variant (grass edge) and a fill variant (dirt only).

### Objects & interaction
```
[stone arch wooden door with lantern | wooden treasure chest with iron bands | blacksmith anvil on wooden table |
 stone forge with fire | wooden bed with red blanket | wooden signpost | hanging red banner with emblem],
single object, side view, sitting on ground line, [style lock]
```

### Plant growth stages (same canvas and base for every stage)
```
farm crop growth stage [1: bare soil mound | 2: tiny sprout | 3: small seedling | 4: leafy plant |
 5: bush with red berries | 6: fully grown bush with many red berries], same soil mound, [style lock]
```

### Nature & decoration
```
[large oak tree with round green canopy | pink blossom tree | mossy boulder | green bush |
 white daisies | red spotted mushrooms | yellow wildflowers | purple flowers | fern | grey rock],
single object, side view, [style lock]
```

### Parallax background (generate each layer separately)
```
layer [1 far: sky with soft white clouds | 2: distant blue mountains with mist |
 3: green hills with waterfall | 4 near: dark forest treeline silhouette],
horizontally seamless, side view, atmospheric perspective, [style lock]
```
Far layers are cooler/bluer with lower contrast (`#93A5B9`, `#3E759D`). Near layers are darker and more saturated.

---

## 8. Consistency check before an asset goes into `assets/`

- [ ] Light comes from the top left, same as the board
- [ ] Colors are close to the palette, with no colors outside it standing out
- [ ] Side view, not isometric
- [ ] Silhouette is readable at 1× scale (zoom out to check)
- [ ] Background is transparent and the canvas size matches section 5
- [ ] Tiles: placed 3×3 with no visible seams
- [ ] Animation: feet stay on the same line in every frame
