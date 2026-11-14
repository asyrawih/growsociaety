# UX · Respawn

Spec for the implementer. Today a stuck player has no way back, and `Collision::Hazard` (lava, item 21, already in
generated worlds) does nothing. Both end the same way: the player is moved to the respawn point.

## Respawn point

The **main door** tile, exactly where a player appears on joining. One point per world. No checkpoints now (see
Do NOT build).

## 1. Manual respawn: "Go home"

| Where | How |
|---|---|
| Keyboard | **Hold `R` for 1 s** (R is unbound today; one `HOME_KEY` constant) |
| Gamepad | Hold **Select/Back** for 1 s |
| Settings panel | A `Go home` row at the top of the panel, with `ui/settings/icon_home.png` (already in the kit); one click, no hold; the panel closes |
| Touch | The same Settings row (no extra on-screen button) |

- **Hold instead of confirm:** while held, a ring fills (1 s), centred on the player's body and drawn in front of it,
  so it never fights the name plate above the head. Releasing early cancels and nothing
  happens. No dialog. A button click in Settings is deliberate enough, so it has no hold.
- **Cooldown 10 s** after any respawn (manual or hazard), so it can't be spammed. During the cooldown the ring does
  not start, and the Settings row is greyed with `Go home (7s)` counting down. Holding R gives the toast
  `You can go home again in 7s` (once per press).
- **What's kept:** everything: inventory, gems, worn items, open trade. There's no combat, so a penalty would only
  punish being stuck. Trades aren't distance-based (doc 08), so respawning can't be used to dodge one. Escaping a
  box someone built around you is exactly what it is for.
- **Server:** a new `Action::Respawn`. The server checks the cooldown and moves the body (its position wins, as
  always). Answer: `Event::Respawned { player, cause: Home | Hazard }` to everyone in the world, so others see the
  player vanish and reappear.

## 2. Hazard touch

- Touching a `Hazard` tile (overlap with the hitbox, checked in `physics::step`, so client and server agree) respawns
  at once. It ignores the cooldown, but it starts one.
- **Seen by the player:** a 0.15 s red flash over the whole screen, the camera cuts to the door, then a 0.25 s fade
  from red back to clear. A small puff at the old spot and at the door, also visible to others.
- **Toast:** lava: `Burned by lava · back at the door`; any future hazard: `Ouch · back at the door`.
  Manual home: no toast and no red flash (the player asked for it): puffs plus a neutral 0.3 s fade (dark at 40 %,
  out and back) to soften the camera jump. Sound for both: the existing door sound.
- **Feel guard:** right after a respawn, ignore hazards for 0.5 s, so a door placed next to lava can't loop.

## Visuals and art

| Need | Asset |
|---|---|
| Hold ring | `ui/respawn_ring.png` (done), 8 × 24×24 strip, frame k = (k+1)/8 filled, centred on the body |
| Puff | `fx/respawn_puff.png` + `.json` (done), 6 × 32×32, 15 fps, no loop, centre pivot |
| Flash and fade | Full-screen colour quad in code, no art |
| Settings row icon | `ui/settings/icon_home.png` (exists) |

## Tests that must exist

- Respawn moves the body to the door spawn position with zero velocity; inventory and gems are unchanged.
- A second `Respawn` within 10 s is refused (`Cooldown`) and changes nothing.
- Stepping onto lava respawns in the same tick on client and server (determinism test).
- No hazard is triggered within 0.5 s after a respawn.

## Do NOT build

- No death screen, no lives, no item or gem loss, no respawn timer you have to wait out after a hazard.
- No checkpoints or beds yet. When they come, they replace the door as the respawn point per player; the flow above
  stays the same.
- No `/home` chat command (the key and the Settings row cover it).
