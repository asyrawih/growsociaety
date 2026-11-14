# UX · Reset world

Spec for d4 (server commands). Destructive and rare, so it is deliberately slow to trigger and loud when it happens.
It sits in the command launcher next to the other staff commands (doc 13) and uses the existing audit path.

## Who

`Requirement::Role(Role::Owner)` (the game owner / super-admins, `GAME_SUPER_ADMINS`). **Not** Admin or Moderator:
nobody but the owner should be able to wipe other players' builds. A world-claim owner resetting their own world
is a later feature (see Do NOT build).

## Commands

| Command | Effect |
|---|---|
| `/resetworld` | Prepare a reset of the world you are in |
| `/resetworld <name>` | Prepare a reset of that world (loaded or not) |
| `… seed=<n>` | Optional: generate with this seed instead of the world's current one |
| `/resetworld confirm` | Carry out the prepared reset, within 15 s, by the same person |
| `/resetworld cancel` | Drop the prepared reset |

## Flow

1. **Prepare** answers privately (System line, amber) with exactly what will go:
   `Reset START? This wipes 12 claims (owned by 7 players), 340 trees, 57 drops and every placed block.
   3 players are inside. Type /resetworld confirm within 15s.`
   The world name in the line is upper-case, the counts are live.
2. **Confirm** within 15 s by the same caller and for the same world. Anything else (another world, someone else,
   too late) answers `Nothing to confirm. Run /resetworld again` and does nothing.
3. **Effect**, in order, inside the world's own task (no other Action interleaves):
   1. Save a **backup** of the current world blob (one row per world, overwritten by the next reset), so a mistake
      can be undone by hand from the DB.
   2. Cancel open trades in that world (`TradeEnded::Invalid`, nothing moves).
   3. **Claims go home:** each claim block becomes one claim item added to its owner's inventory (online or
      stored). If the owner's inventory has no room, refund the store price in gems instead
      (`claims::CLAIMS` price). Nothing of value is lost. That owner gets a private line (now, or on next login):
      `Your Small Claim in START was returned because the owner reset the world.`
   4. Regenerate terrain with `worldgen` (same name, seed as given), which removes every tree, drop and placed block.
   5. Every player inside is moved to the new main door (respawn.md, cause `Home`, no cooldown started) and sees the
      toast `This world was reset by the owner`.
4. **Audit line** (existing `audit`): who, world, seed, and the counts actually wiped (recounted at confirm, not the
   prepare-time numbers).
5. The caller gets `START was reset (12 claims returned, 340 trees, 57 drops removed).`

## Not touched

Player inventories (except returned claims), gems (except refunds), worn items, accounts, levels and XP, other
worlds, the access lists of claims in other worlds.

## Do NOT build

- No reset for Admin/Moderator, no button anywhere in the UI (launcher only), no scheduled or automatic resets.
- No world-claim owner reset yet. If it comes later: owner of the world claim only, the same confirm, a 24 h cooldown,
  and it must still return every other player's claims.
- No partial resets (area, layer) and no "undo" command (the backup row is for manual recovery).

## Tests that must exist

- Prepare without confirm changes nothing; confirm after 15 s, by someone else, or for another world does nothing.
- After a reset: every claim owner holds one more claim item (or got exactly the refund), the world has no trees,
  drops or claims, and each player inside is at the new door.
- A non-Owner caller is refused and audited like other role commands.
