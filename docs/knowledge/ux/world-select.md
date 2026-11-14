# UX · World select (create and enter worlds)

Spec for 1a (client), d4 (server) and the lobby. The rules already exist (doc 02: 1–24 letters/digits, stored
upper-case, a server deny-list, an unused name generates a new world, 64 players per world). Only the way in is missing.

## The World menu

One panel (`ui/panel.png`), shown **after login** (as the lobby's last step, before the game starts) and **in game**
from the main door, the top-bar World button (`ui/settings/icon_world.png`, exists, tooltip `Worlds (M)`), the
**M** key (unbound today) and `/warp <name>`.

```
┌ Worlds ─────────────────────────────────────── [x] ┐
│ [ Enter world name______________ ]  [  Go  ]        │
│   MYFARM doesn't exist yet: entering it creates it  │  ← live status line
│ Recent                     Active now               │
│  MYFARM        just you     START        12/64      │
│  BUDIHOUSE     3/64         TRADEHUB     30/64      │
│  …                          …                       │
│                            [ Random world ]         │
└─────────────────────────────────────────────────────┘
```

| Part | Behaviour |
|---|---|
| Name field | Upper-cases as you type; characters that aren't letters/digits are not inserted; stops at 24. Placeholder `Enter world name`. Enter = Go |
| Status line (300 ms after typing stops; one server lookup) | `START · 12 players` · `MYFARM doesn't exist yet: entering it creates it` · `START is full (64/64)` · `That name isn't allowed` |
| Go button | Label `Go`, or `Create` when the name is new; disabled while empty or the status says full/not allowed |
| Active now | The 10 busiest loaded worlds, name + `12/64`; click enters. Full ones greyed |
| Recent | The player's last 5 worlds (server-stored per account) with their current count (`just you`, `3/64`, `empty`) |
| Random world | Enters a random active world that has 1–60 players; hidden when there is none |

**No confirm for creating:** a new world is free, harmless and identical every time for the same name (worldgen is a
hash of the name), so the status line and the `Create` label are warning enough.

## Errors (toast in the panel, `denied` sound)

| Case | Text |
|---|---|
| Deny-list | `That name isn't allowed` |
| Full | `<NAME> is full (64/64). Try another world` |
| Creation limit | `You've opened 10 new worlds this hour. Visit an existing one or try again later` |
| Server unreachable / timeout | `Couldn't reach the server. Try again` |

## Entering a new world

When you enter a world that was just created by you and has no world claim, show one hint (the first-run-hints box,
not a toast): `This world is new. Buy a World Claim (B) to make it yours · Learn more (J)`. `J` opens the Almanac
claims page. Shown once per world, gone after 10 s or on any action.

## Leaving and warping

- **Main door:** using it opens the World menu. The player stays in the current world until they pick another;
  Esc/✕ closes the menu and nothing happens. (Today the door sends `Punch` and gets `LeftWorld`. The client should
  open the menu instead of sending the punch, and send `Leave` + `Join` only on Go.)
- `/warp <name>` = the same as typing the name and pressing Go (same errors, same creation rule).
- Switching shows a 0.3 s fade, then the new world.

## Limits

- **Creation:** at most 10 new worlds per account per hour (server). Entering existing worlds is unlimited.
- **Untouched worlds aren't stored:** a world that nobody changed regenerates identically from its name, so it is
  only persisted once a block or claim changes. Name-squatting by visiting costs the server nothing.
- Reserved names: the deny-list plus `START` is never resettable by world claims (it's the hub).

## Server needs (d4)

| Need | Shape |
|---|---|
| World list | `ClientMsg::ListWorlds` → `ServerMsg::Worlds { active: Vec<(String, u8)>, recent: Vec<(String, u8)> }` |
| Name lookup | `ClientMsg::WorldInfo { name }` → `ServerMsg::WorldInfo { name, exists, players, allowed }`, rate-limited |
| Recent store | DB `recent_worlds(player_id, world, last_at)`, last 5 per account, written on Join |
| Switching | `Leave` then `Join { world }` on the same connection (conn.rs already allows Join after Leave) |
| Creation limit | Per account, counted on the first persist of a new name |

## Input

- Keyboard: the menu opens with the field focused (`TextFocus`); ↓ moves into the lists; Enter goes; Esc closes
  (in game) and does nothing on the after-login screen (there's nowhere to go back to except logging out).
- Gamepad: d-pad through Recent / Active / Random; A enters; B closes. No text entry on gamepad.
- Touch: the field opens the on-screen keyboard; rows ≥ 44 px.

## Do NOT build

- No world browser with search/sort/pages, no world descriptions, thumbnails or ratings.
- No private/password worlds (claims already control building; entering is always allowed, doc 07).
- No deleting worlds from the client.
