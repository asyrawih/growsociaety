# UX · World Deed (transfer or sell a world)

Spec for 1f (server), 8e (client) and 6e (art). Our item is the **World Deed**: a signed paper that hands over a
world's World Claim. Original name, art and flow.

## Why not a normal item

Inventory slots are `(ItemId, count)` with no per-item data, so two deeds for two worlds would stack into one
meaningless "×2". A deed must always say which world it is for. So deeds live on a **deed ring**, a short list on the
player (`deeds: Vec<WorldName>`), shown in the bag and tradeable as their own trade rows. No inventory or stacking
rule changes.

**Where the ring shows:** in the bottom-HUD bag (bottom-hud.md), as a last chip `Deeds (N)` after Misc, shown only
while the player holds at least one deed. Selecting it replaces the grid rows with the deed list (one row per deed:
icon, world name, `Use` · `Tear up`); the hotbar row stays. Search and Sort are hidden while it is selected.
The Make/Use/Tear up buttons and their confirms are in the keyboard/gamepad focus order, like every panel button.

## Make a deed

1. The world claim's owner wrench-clicks the World Claim. The claim panel (claims.md §5) gets a button
   `Make World Deed` (owner only, world claims only).
2. Confirm in the panel: `Make a deed for MYFARM? Whoever uses it becomes the owner. You stay the owner until then.`
   [Make deed] [Cancel].
3. The deed appears on the owner's ring. **One deed per world at a time**: while one exists, the button reads
   `Deed out` (disabled) and the claim panel shows `A deed for this world exists` **to everyone**, visitors included,
   so a buyer can check a deed is real before trading.
4. The world stays fully protected under the current owner until the deed is used.

## Use a deed

- From the deed ring (bag): select the deed → `Use` (works from anywhere, not only inside that world).
- Confirm: `Become the owner of MYFARM? The deed is used up.` [Use deed] [Cancel].
- **Effect (server, one transaction):** the world claim's owner becomes the user, its **access list is cleared** (a
  bought world starts clean, so the seller keeps no hidden builders), `public` is turned off, the deed is removed.
  Area claims other players own inside the world stay theirs (doc 07).
- New owner toast: `You now own MYFARM`. Old owner gets a private line now or at next login:
  `MYFARM now belongs to <name> (your deed was used).`

## Cancel

The owner uses their own deed → `Tear up the deed for MYFARM? You stay the owner.` [Tear up] → deed removed,
nothing else changes. A holder who isn't the owner sees `Tear up the deed for MYFARM? It can't be used afterwards.
<owner> stays the owner.` instead (the owner sentence only when the client knows the owner, i.e. inside that
world; elsewhere it's left out). The ring also has a `Tear up` button on every deed (same confirm). Deeds can't be dropped or
trashed: tearing up is the only way to throw one away.

## Void deeds

If the world is reset (reset-world.md), its World Claim is broken, or its owner changes another way, every deed for it
becomes **void**: still on the ring, greyed, with the tooltip `Void: MYFARM's World Claim is gone` and only a
`Discard` button. Using a void deed is refused with that same text.

## Trading

- The trade window gets a `Deeds` row under the item grid; you add deeds from your ring.
- The other side sees each deed as a large line, not an icon: `World Deed: MYFARM`, plus the warning
  `Check the world name. Deeds can't be undone.` Your own offered deeds show at normal size with a remove `x`.
- Any change to the deeds offered unlocks both sides, like items (doc 08).

## Tooltip, Almanac, audit

- Deed tooltip (U3 box): `World Deed · MYFARM` / `Use it to become the owner of MYFARM` (or the void text).
- Almanac Basics page "World Deeds": make, trade, use, tear up, void (generated sections untouched).
- One audit line each for make, use, tear up and void, with world and player ids.

## Server work (1f)

| Need | Shape |
|---|---|
| Deed ring | `Player.deeds: Vec<String>` (persisted per account), private to its owner |
| Registry | Table `world_deeds(world PK, holder_id, made_by, made_at)`: enforces one deed per world and finds the holder when voiding |
| Actions | `MakeDeed { x, y }`, `UseDeed { world }`, `TearUpDeed { world }`; trade offers gain `deeds: Vec<String>` |
| Checks | Make: caller owns the world claim, no deed exists. Use: deed on the caller's ring, still valid. Void on reset, claim break or owner change |

## Art (done by 6e: `ui/deed.png` 32×32, `ui/deed_small.png` 16×16 drawn natively)

`ui/deed.png`: 32×32 rolled parchment with a wax seal in our palette, original; plus a 16×16 version for the trade
row and ring. Void deeds use the same art greyed in code.

## Do NOT build

- No deeds for area claims (only World Claims).
- No deed expiry, auctions or price tags, no copying a deed.
- No remote ownership changes besides using the deed.
