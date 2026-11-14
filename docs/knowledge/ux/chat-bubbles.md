# UX · Chat bubbles

Spec for 8e (client only). When a player says something in **world chat** (`ChatKind::World`), a speech bubble shows
it above their head. Private (`/msg`, `/r`) and System lines never get a bubble. The chat log is unchanged.

## Look

```
      ┌──────────────────────┐
      │ anyone selling dirt  │      ← ui/bubble.png 9-slice, INK_COLOR text
      │ seeds? paying well   │
      └──────────┬───────────┘
                 ▼                  ← tail from the same art, centred, points at the head
               Budi                 ← existing name plate
              (rig)
```

- Text: the pixel font at `NAME_FONT_SIZE` screen pixels, with the same counter-scaling as names (`player.rs`), so
  the bubble reads the same at every zoom.
- Max width 220 screen px; wraps on words (long words break). At most **3 lines**; a longer message ends in `...` on
  line 3 (the full text is in the chat log).
- Sits above the name plate (the name plate never moves), 4 px gap. Follows the player every frame.

## Lifetime

`3 s + 0.06 s per character`, clamped to **3–6 s**. Fades out over the last 0.3 s, fades in over 0.1 s.

## Rapid messages

- One bubble per player. A new message from the same player **replaces** the bubble text immediately and restarts
  its lifetime (no stacking towers).
- Overlapping players: bubbles don't avoid each other (rare and short-lived); the newest is drawn on top.

## Offscreen and far

- A speaker outside the camera view gets no bubble and no edge arrow; the chat log has it.
- Bubbles for players more than 20 tiles from the camera centre are skipped (at the furthest zoom out they'd be
  noise).

## Muting and settings

- Players you muted (avatar-menu.md: Mute = server-side ignore) never send you their lines, so no bubbles either.
- **Settings > Display > `Chat bubbles`** toggle (next to Reduce effects; no new tab), on by default. Off hides all bubbles, the log is unchanged.
- Your own messages get a bubble too (you see what others see).

## Content rules

Bubbles show exactly the text the server broadcast, already cleaned by `clean_chat` (length cap `MAX_CHAT_LEN`,
control characters stripped, profanity masked). The client never re-filters or re-formats it.

## Art (done by 6e)

| Asset | Spec |
|---|---|
| `ui/bubble.png` | 32×24, 9-slice inset 6, parchment + dark 1 px outline, rounder corners than the tooltip |
| `ui/bubble_tail.png` | 10×6, points down. Place it centred under the box and **1 px up**, overlapping the bubble's bottom outline row, so they join without a seam |

## Do NOT build

- No bubbles for whispers, system lines, commands or emotes; no typing indicator (`...` while typing).
- No bubble history, no clickable bubbles.
- No voice-activity bubbles (the speaking indicator already exists).
