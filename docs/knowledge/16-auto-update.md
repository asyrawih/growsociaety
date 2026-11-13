# 16 · Auto-update

Plan, not built. Written 2026-11-14. Today a release is a tagged GitHub Actions build that zips the desktop binary
with `assets/` (and a `.app` on macOS); players download it by hand, and the server refuses a client whose
`PROTOCOL_VERSION` (now 8) differs, with no explanation. This plan makes updates reach players reliably and safely on
every platform.

## Knowledge

### Three layers

| Layer | Platforms | What it does |
|---|---|---|
| **1. Version gate** | All | The server knows which client versions it accepts and tells an old client what to do, instead of a bare refusal |
| **2. Self-update** | Windows, macOS, Linux | The desktop app downloads, verifies and installs new versions itself |
| **3. Store update** | iOS, Android | The stores deliver updates; the app only checks and points the player there |

Layer 1 is the foundation: it is what actually forces an update when one is required, on every platform.

### Layer 1: version gate (server decides)

- `Hello` carries the client's **app version** (semver, e.g. `0.4.2`), its **platform**, and `PROTOCOL_VERSION`.
- The server holds, per platform, a `latest` version, a `minimum` version and a download/store URL, in server config
  (not compiled in), so they change without a server rebuild.
- The server answers before login:

| Client version | Server answer | Client shows |
|---|---|---|
| ≥ `latest` | Proceed | Nothing |
| ≥ `minimum`, < `latest` | Proceed + `UpdateAvailable { latest, url }` | A small, dismissible notice in the lobby ("Update available — restart to update" on desktop, "Update in the store" on mobile) |
| < `minimum`, or a protocol it cannot speak | `UpdateRequired { minimum, url }` instead of a refusal | A blocking screen: desktop downloads and restarts; mobile opens the store page |

- **Protocol window.** Where a change allows it, the server keeps accepting the previous `PROTOCOL_VERSION` for a
  while, so a release does not cut off everyone who has not updated yet. When a change cannot be bridged, raise
  `minimum` so old clients get `UpdateRequired` rather than a confusing failure.
- **Release order:** (1) deploy a server that accepts old and new clients, (2) publish the new clients, (3) later raise
  `minimum`. Never ship a client that needs a server that is not deployed yet.

### Layer 2: desktop self-update

**Recommended tool: Velopack.** It is written in Rust, has a Rust crate, and builds the installer, full packages and
**delta** packages (small downloads) for Windows, macOS and Linux from `cargo build` output in one command (`vpk`).
The app uses its `UpdateManager` to check, download and apply with a restart. On macOS, Velopack performs the code
signing and notarization during packaging (its helper binaries must be signed too); on Windows it supports signing in
CI (e.g. Azure Trusted Signing).

Alternative considered: the `self_update` crate with `zipsign` signatures (ed25519, public key embedded in the
binary). Simpler, but it replaces a single binary: no installer, no delta updates, and replacing a signed macOS `.app`
bundle in place is awkward. Use it only if Velopack does not fit.

Flow:

1. The lobby checks the update feed in the background at start (never blocks login).
2. If an update exists, it downloads in the background (delta when possible) and shows "Update ready — restart".
3. Applying happens on restart, never in the middle of a game session.
4. If the server says `UpdateRequired`, the lobby shows a blocking screen with progress, downloads, applies and
   restarts.
5. Channels: `stable` for players, `beta` for testers, chosen in Settings (default stable).

Hosting the feed: GitHub Releases (the CI already publishes there; only works without credentials if the repository
or its releases are public) or static files on our own server behind HTTPS (nginx on the production host, like
LiveKit). Decision for the owner.

### Layer 3: mobile store updates

Both stores forbid an app from replacing its own executable code: updates come through the store.

| Platform | Optional update | Required update |
|---|---|---|
| Android | Google Play **In-App Updates** API, *flexible* flow (downloads in the background, the app triggers install) | *Immediate* flow (full-screen, Play installs and restarts). Update priority 0–5 can be set per release through the Play Developer API |
| iOS | A notice with a link to the App Store page | A blocking screen with the App Store link. Apple offers no way to force an update; enforcement is the server's `minimum` (layer 1). The iTunes lookup endpoint is common practice for reading the store version but is not documented as supported; prefer the server's `latest` |

### Content without an app update

All content (items, recipes, sprites, sounds) ships inside the app: the item table is compiled into `game_shared`, so a
new item needs a new build. Downloading art or audio packs later is possible (stores allow downloading assets, not
code), but there is no need yet. Not in this plan.

### Security (non-negotiable)

An updater is a remote code installer: if an attacker can feed it a package, they own every player's computer.

| Rule | How |
|---|---|
| Packages are signed and verified before install | macOS: Developer ID signing + notarization. Windows: Authenticode signing. The updater verifies the package hash from the release feed; with `self_update`, an ed25519 signature checked against a public key embedded in the binary |
| Feed over HTTPS only | No HTTP fallback |
| Signing keys only in CI secrets | Releases built only by CI from a tagged commit on `main`; nobody signs on a laptop |
| No downgrade by a feed | The updater only moves forward; a bad release is fixed by a newer one (roll forward) |
| The version gate is not a security boundary | A modified client can lie about its version; it only helps honest players. Security stays in server validation (doc 10) |

### Release pipeline (target)

```
tag vX.Y.Z on main
  → CI builds the client for each desktop target (release profile)
  → vpk pack (installer + full + delta), signed and notarized per OS
  → upload to the release feed (stable or beta channel)
  → mobile: build, sign, upload to TestFlight / Play internal track, then promote
  → update server config: latest = X.Y.Z (and minimum, when a breaking change requires it)
```

The app version comes from the workspace `version` in `Cargo.toml` (now `0.1.0`), injected at build time; the git tag
must match it (CI checks).

## Technical

### Messages (extends doc 10)

```rust
ClientMsg::Hello { protocol: u16, app_version: String, platform: Platform, auth: Auth, .. }
enum Platform { Windows, MacOS, Linux, Ios, Android }

ServerMsg::UpdateAvailable { latest: String, url: String }        // sent with a normal Welcome
ServerMsg::UpdateRequired { minimum: String, url: String }        // sent instead of Welcome; then the server closes
```

Server config (per platform): `latest`, `minimum`, `url`. Version comparison is semver, parsed and validated at the
boundary; an unparsable client version is treated as below `minimum`.

### Client

- A small `update` module: background feed check in the lobby (Velopack `UpdateManager` on desktop, Play In-App
  Updates through JNI on Android, store link on iOS), state `None | Available | Downloading(progress) | Ready |
  Required`, and the lobby UI for each state (notice, progress, restart button, blocking screen).
- `--no-default-features` and development builds skip the updater (no feed, no prompts).
- The updater never runs in screenshot mode or offline mode.

### Ownership (who would build it)

| Part | Natural owner |
|---|---|
| `Hello` fields, `UpdateAvailable`/`UpdateRequired`, server config, protocol window | Coding session (protocol, server) |
| Lobby update UI and the client `update` module | growsociaety-75 (lobby) |
| CI packaging with Velopack, signing, notarization, feed upload | Deployment session (growsociaety-f8) or the coding session |
| Mobile store flows | With the mobile work (doc 15) |

### Tests that must exist

- Version gate: a client at `latest` proceeds silently; between `minimum` and `latest` gets `UpdateAvailable`; below
  `minimum` gets `UpdateRequired` and no `Welcome`; an unparsable version is treated as below `minimum`; per-platform
  values are respected.
- Semver comparison (pre-release tags, missing parts).
- Client update state machine transitions, including "Required" blocking the lobby and "Ready" applying only on restart.
- Manual, per release: update from the previous version on each desktop OS (delta and full), a forced update path,
  a corrupted download is rejected, and an update over a slow connection does not block login.

## Open decisions for the owner

| # | Question | Default in this plan |
|---|---|---|
| U-D1 | Desktop updater | Velopack |
| U-D2 | Where the update feed is hosted | Own server over HTTPS (works whether the repository is public or private) |
| U-D3 | Signing accounts: Apple Developer ID (also needed for iOS), a Windows code-signing option | Owner's accounts; Windows via Azure Trusted Signing |
| U-D4 | Channels | `stable` + `beta` |
| U-D5 | How long the server keeps accepting the previous protocol version | Until the next release, when the change allows it |
| U-D6 | Linux packaging | Velopack's Linux package (AppImage) instead of the current zip |
