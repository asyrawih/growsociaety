# UX · Avatar menu (click a player)

Spec for 8e (client) and 1f (server gaps). A small context menu next to a player, so social actions don't need chat
commands.

## Opening it

Left click punches/places, so it can't open menus.

| Device | Open |
|---|---|
| Mouse | **Right-click** on a player's body (their rig's hit box, a little larger than the 20×44 hitbox). Also a left click while holding the **wrench** (the "inspect" tool, like claims) |
| Touch | **Long-press 400 ms** on a player. Player hit-testing goes before tiles, so pressing a player standing on a tree opens this menu, not the tree card |
| Gamepad | Later (needs the gamepad cursor pass) |

Distance doesn't matter: anyone on screen can be clicked.

## Layout

```
┌────────────────────────────┐   ← ui/tooltip.png box, anchored beside the player, kept on screen
│ 🇮🇩 Budi          Lv 12     │   header: flag, name, level
│ ────────────────────────── │
│ Trade                      │
│ Whisper                    │
│ Mute                       │   ← "Unmute" when muted
│ Report                     │
│ ────────────────────────── │
│ Kick            (owner/mod)│   role-gated rows, only when allowed
│ Ban from world  ▸ 30m 2h 7d│
└────────────────────────────┘
```

Rows are 32 px tall (44 px on touch), hover = parchment highlight, focus frame for keyboard.

## Actions

| Row | Does | Exists? |
|---|---|---|
| Trade | `TradeRequest` to them (same as `/trade <name>`) | Yes |
| Whisper | Opens the chat line pre-filled `/msg <name> ` with focus | Yes (`/msg`) |
| Mute / Unmute | Server-side ignore (`/ignore` / `/unignore`) **and** the local per-player voice mute ("mute for me"), never the world-wide `Net::voice_mute`: their chat, bubbles and voice disappear for you only | Yes, both. Staff can't be muted: row greyed, tooltip `Staff can't be muted` |
| Report | Opens a 3-choice picker: `Spam` · `Rude` · `Cheating`, then sends; toast `Thanks, staff will look at it` | **No** (server gap) |
| Kick | `/kick <name>`; only shown if you own the world or have World access (the command's own rule) | Yes |
| Ban from world | `/wban <name> <30m\|2h\|7d>` via the three inline choices; the first click turns the chosen one red and reads `Ban 2h?`, a second click within 3 s bans | Yes |
| View profile | Later | No |

Rows the server would refuse are hidden, not greyed (the client knows the role and world ownership); the server
still decides.

## Yourself

Right-click (or long-press) yourself: a short menu, header with your own name/level, rows `Wardrobe` · `Go home`
(respawn.md) · `Almanac`. No social rows.

## Closing and states

- Opening it closes any open centre panel (one panel at a time).
- While open it takes Up/Down/Enter (the player doesn't move or jump), like the store; Esc closes.
- Closes on: Esc, a click outside, choosing a row, or opening another panel.
- The player moves: the menu follows them while open.
- They leave the world: rows grey out and the header reads `Budi left`; closes after 1.5 s.
- They're too far for an action that needs range (none today): shown greyed with the reason, never hidden.

## Server gaps for 1f

Already there (no work): `/trade`, `/msg` and `/r`, `/ignore` and `/unignore` (per world session), `/kick`, `/pull`,
`/wban`, `/wunban`, `voice_mute`, `Net::role()`, names via `Net::name` / `lookup_names`. World authority (world-claim
owner or access list) is known on the client from the mirrored world claim.

| Gap | Need |
|---|---|
| Report | `ClientMsg::Report { player, reason: Spam\|Rude\|Cheating }`. Table `reports(id, reporter_id, target_id, world, reason, chat jsonb, created_at)` where `chat` = the world's last 20 World/Private lines the reporter received (speaker id, text, time). One audit-log line per report until a staff UI exists. 5 per account per hour; ack only, never tells the target |
| Persistent mute | Table `ignores(player_id, ignored_id)`; loaded at login, applied in every world; `/ignore` and `/unignore` write it. Staff still can't be ignored |

## Art

None new: `ui/tooltip.png` box, existing flags, the pixel font.

## Do NOT build

- No friends list, no player profile page, no gifting from the menu, no "follow" or teleport-to.
- No report text field (free text invites abuse; the chat log is attached).
