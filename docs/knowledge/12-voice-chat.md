# 12 · Voice Chat (LiveKit)

Plan, not built. It depends on M7: voice needs the game server to know who is in which world.

## Knowledge

### What players get

- Voice with the people **in the same world**, nobody else.
- **Proximity:** you hear players near you clearly, farther ones quieter, and players across the map not at all.
  This fits small worlds: a crowded trade spot does not become one noisy call.
- **Push-to-talk or open mic**, chosen in a voice settings menu (push-to-talk = hold **V**).
- A speaking indicator above a player's head, so you know who is talking.
- Mute yourself, mute any other player for yourself, a voice volume slider, and an input device picker.
- Voice is **off until the player turns it on** the first time. Nobody's microphone opens by surprise.

### Design rules

| Rule | Why |
|---|---|
| One LiveKit room per game world, named `world-<NAME>` | Leaving the world leaves the voice room; there is no separate voice UI to manage |
| The game server mints LiveKit tokens; the client never sees the API key or secret | A client with the secret could join any room as anyone |
| The LiveKit identity is the game `PlayerId` | Voice can be tied to the player on screen, and moderation acts on the same id |
| Voice is a view: it reads player positions, it never changes game state | Same rule as rendering ([09](09-architecture.md)); voice failing must never break play |
| Offline single-player has no voice | There is no server to issue a token |
| A voice failure (no mic, LiveKit down, token refused) shows one message and the game continues | Voice is optional |

### Proximity curve (all Tuning)

| Constant | Value | Effect |
|---|---|---|
| `VOICE_NEAR` | 6 tiles | Full volume inside this distance |
| `VOICE_FAR` | 16 tiles | Volume falls linearly to 0 here |
| `VOICE_UNSUBSCRIBE` | 18 tiles | Beyond this the track is unsubscribed (no bandwidth used) |
| `VOICE_RESUBSCRIBE` | 16 tiles | Closer than this it is subscribed again. The gap stops flapping at the boundary |
| `VOICE_PAN` | ±0.6 at `VOICE_FAR` horizontally | Light stereo panning by horizontal offset; never fully one-sided |

Distance is measured between the two players' interpolated body positions, which the client already has.

Known limit: proximity is enforced by the **client**. Everyone in the world's room is allowed to subscribe to everyone,
so a modified client could hear the whole world. That is acceptable while worlds are public spaces. If private
conversation ever matters, move subscription control to the server (tokens with subscribing disabled, and the server
deciding subscriptions through LiveKit's server API). Verify that API before relying on it.

### Moderation

| Who | Can | How |
|---|---|---|
| Any player | Mute anyone for themselves | Client-side, by not playing that track |
| The world claim owner | Server-mute a player's voice in their world | Game server calls LiveKit's server API to mute that participant's published track |
| The server | Remove a player from voice when they are kicked or leave | Game server removes the participant from the room |

Reporting, recording and age gating are out of scope for this plan. Raise them with the owner before voice reaches
real players: voice in an open sandbox reaches strangers, including minors.

## Technical

### Components

```
                       ┌──────────────── game server (world_server) ────────────────┐
  client ──Join──────▶ │ player joins world  ─▶ mint LiveKit token (livekit-api)     │
         ◀─VoiceToken─ │   identity = PlayerId, room = world-<NAME>, TTL 10 min      │
                       │ world owner mutes / player kicked ─▶ LiveKit server API     │
                       └─────────────────────────────────────────────────────────────┘
  client ──WebRTC────▶ LiveKit server (self-hosted livekit-server, or LiveKit Cloud)
     │
     ├─ mic ─▶ local audio track (publish only while push-to-talk is held, or always on open mic)
     └─ remote tracks ─▶ per-speaker gain (proximity) ─▶ mixer ─▶ speakers
```

### Crates

| Crate | Where | For |
|---|---|---|
| `livekit-api` | `server/world_server` only | `AccessToken` + `VideoGrants` to mint tokens; the server API client for mute and remove |
| `livekit` | `client/game_client` only | Connecting to the room, publishing the mic, receiving remote audio |

`game_shared` gets nothing. Voice has no rules.

Verify both crates' current versions and APIs at implementation time; their grant structs and audio APIs have
changed between releases. Specifically check:

- **Microphone capture.** Recent `livekit` releases offer a platform audio mode that captures the microphone through
  WebRTC's own audio device module. Prefer it: it brings echo cancellation, noise suppression and gain control. The
  alternative (capturing with `cpal` and pushing frames into a `NativeAudioSource`) has no echo cancellation, so
  players without headphones would feed the call back into itself.
- **Playback.** Remote audio arrives as frames per track. To apply the proximity gain the client must control
  playback per speaker: either the SDK's per-track volume, if the version has one, or reading the frames and mixing
  them itself into one output stream. Pick whichever the chosen version supports; the gain function is the same.

### Messages (extends [10](10-networking-and-persistence.md))

```rust
ServerMsg::VoiceToken { url: String, token: String, expires_in: u32 }  // sent after Joined, and re-sent before expiry
ServerMsg::VoiceMuted { player: PlayerId, muted: bool }                 // so clients can show a muted icon
ClientMsg::VoiceMute { player: PlayerId, muted: bool }                  // world claim owner only; server checks
```

The server mints a token only for a player who is currently in that world, with:

| Grant | Value |
|---|---|
| `room_join` | true |
| `room` | `world-<NAME>` of the world the player is in |
| `identity` | the player's `PlayerId` as a string |
| `can_publish` | true, limited to microphone audio if the version supports source limits |
| `can_publish_data` | false (game data goes through the game server, never LiveKit) |
| `can_subscribe` | true |
| TTL | 10 minutes; the server re-sends a fresh token before it expires while the player stays |

`VoiceMute` follows the usual boundary rules: identity from the connection, ownership checked with the same claim
data as `can_edit`, rate-limited.

### Client module

`client/game_client/src/voice.rs`, one Bevy plugin:

- LiveKit runs on Tokio; Bevy does not. Run a Tokio runtime on a background thread (reuse the one `net.rs` runs, if
  it has one) and talk to Bevy through channels: Bevy sends "connect / disconnect / publish on / publish off /
  set gains", the voice thread sends "connected / failed / speaking levels / participant joined or left".
- Every ~100 ms, compute each remote speaker's gain and pan from the bodies in `WorldState` and send them across.
- Push-to-talk: holding **V** enables the local track, releasing disables it. Respect the "pointer over UI" and
  screenshot-mode input rules the rest of the client uses.
- Speaking indicator: from LiveKit's active-speaker or audio-level events, drawn above the player's head in
  `world_view` (owner of that file decides placement).
- Settings (mute self, per-player mute, volume, input device, push-to-talk vs open mic): stored locally on the
  machine; nothing goes to the game server except `VoiceMute` from world owners.
- Leaving or switching worlds disconnects from the old room before joining the new one.

The gain function is pure and lives in `voice.rs`:

```rust
fn proximity(dx_tiles: f32, dy_tiles: f32) -> (gain: f32, pan: f32)
```

### LiveKit server

| Option | Good for | Cost and work |
|---|---|---|
| Self-hosted `livekit-server` | Development (run it locally with Docker) and later production | Free software; needs a host with UDP ports open and a TURN setup for players behind strict NATs |
| LiveKit Cloud | Production without running media servers | Billed by usage (participant minutes and bandwidth) |

Development server: `infra/livekit/` (Docker Compose, image `livekit/livekit-server:v1.13.10`).

```sh
cd infra/livekit && docker compose up -d      # stop: docker compose down
```

| Setting | Development value |
|---|---|
| `LIVEKIT_URL` | `ws://localhost:7880` |
| `LIVEKIT_API_KEY` | `devkey` (override with the same env var when starting compose) |
| `LIVEKIT_API_SECRET` | `devsecret-change-me-at-least-32-chars` (override likewise) |
| Ports | 7880/tcp signalling and server API, 7881/tcp ICE fallback, 7882/udp media |
| Testing from another device | `LIVEKIT_NODE_IP=<LAN IP> docker compose up -d`; otherwise only this machine can carry media |

Media uses one UDP port (UDP mux) instead of a 10,000-port range, which Docker Desktop and OrbStack publish slowly.
Rooms auto-create, close 60 s after emptying, and hold at most 64 participants (`MAX_PLAYERS_PER_WORLD`).

The game server reads `LIVEKIT_URL`, `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` from its environment. The development
values above are for local use only; real secrets are never committed, logged or sent to a client.

### Build and platform impact

- The `livekit` client crate links WebRTC (downloaded as a prebuilt library). Expect a noticeably bigger client
  binary and longer clean builds. CI needs caching for it and may need extra Linux packages; check the crate's README.
- Put voice behind a client cargo feature (`voice`, on by default) so a build without WebRTC stays possible if it
  ever blocks a platform.
- macOS asks for microphone permission. Running from a terminal uses the terminal's permission; a bundled app needs
  `NSMicrophoneUsageDescription` in its `Info.plist`.
- Mobile (README principle 9): check the LiveKit Rust SDK's iOS and Android support before promising voice there.

### Phases

| Phase | Delivers | Needs |
|---|---|---|
| V0 | Local `livekit-server`, token minting in the game server, `VoiceToken` sent after `Joined` | M7 join flow |
| V1 | Client joins the world's room, push-to-talk publishes, everyone in the world hears everyone at full volume | V0 |
| V2 | Proximity gain and pan, unsubscribe beyond range with hysteresis | V1 |
| V3 | Speaking indicator, voice settings panel, per-player mute, device picker, first-use opt-in | V1 |
| V4 | World-owner mute, removal on kick or leave, token refresh | V0, claims (M6) |

V1 is the first point where two people can talk; V2 is when it starts to feel like part of the world.

### Tests that must exist

- Token minting: the decoded token has the right room, identity, TTL and grants; no token is minted for a player
  who is not in that world.
- `proximity`: full gain at 0 and at `VOICE_NEAR`, 0 at `VOICE_FAR` and beyond, monotonic in between; pan is 0 at
  dx = 0, symmetric, and never exceeds `VOICE_PAN`.
- Subscription hysteresis: a speaker oscillating around `VOICE_UNSUBSCRIBE` does not toggle on every update.
- `VoiceMute` from a player who does not own the world claim is rejected.
- The game keeps running (no panic, one message) when the LiveKit URL is unreachable or the token is refused.
- Manual: two clients in one world can talk; walking apart fades the voice; push-to-talk opens and closes the mic.

## As built (2026-11-13)

Reported by the scripting subagent; not QC-reviewed (QC happens at the end).

| Topic | As built |
|---|---|
| Crates | `livekit =0.10.0`, `livekit-api =0.10.0`; `cpal 0.17` for audio devices. Client feature `voice` (default on); `cargo build -p game_client --no-default-features` builds without WebRTC |
| macOS link | `client/game_client/build.rs` adds `-ObjC` on macOS when `voice` is on; without it WebRTC crashed at connect |
| Server | `server/world_server/src/voice.rs` mints tokens (room `world-<NAME>`, identity = PlayerId, TTL 600 s, publish limited to microphone, no data). `VoiceToken` right after `Joined`, refreshed for everyone every 300 s from a timer inside the world task. Voice is off (logged once) if `LIVEKIT_URL`/`_API_KEY`/`_API_SECRET` are unset |
| Mic path | cpal capture → WebRTC software audio processing (echo cancellation with the playback mix as reference, gain control, high-pass, noise suppression) → `NativeAudioSource`. The SDK's platform-audio mode was not used: it plays every remote track itself with no per-track volume, which rules out proximity |
| Playback | Each remote track read as frames and mixed by the client with per-speaker gain (smoothed) and pan into one cpal output; remote queues capped at 200 ms |
| Proximity | Doc constants as given; updated every 100 ms; unsubscribe/resubscribe with hysteresis |
| Push-to-talk / open mic | Push-to-talk starts the OS microphone stream on V press and stops it on release (buffers cleared, echo canceller kept warm); open mic keeps it running while connected (V-D6) |
| Settings | The **Voice tab of the Settings window** (key **N**, the top-bar mic button, or the gear): voice on/off (default off), push-to-talk or open mic (one choice, two toggles), mute self, microphone dropdown, per-player "Mute for me", and "Mute for all" for the world claim owner or staff. Voice volume is on the Audio tab. All settings live in one file, `settings.txt` in the OS config dir (an old `voice.txt` is migrated once); per-player mutes stay in memory |
| Indicators | Mic status line top-left; speech bubble over talking players; muted-mic icon over owner-muted players. Uses the `assets/ui/voice_*` icons (not `voice_level_bars`) |
| Owner mute (V4) | `ClientMsg::VoiceMute`, world claim owner or staff, never on an equal-or-higher role (access holders refused), rate-limited at the connection. The server revokes the participant's publish permission and later tokens carry no publish grant; `VoiceMuted` broadcast and sent to newcomers |
| Leaving | `Room::remove` removes the participant from the LiveKit room and revokes older tokens |
| Testing without a mic | Debug-only `GAME_VOICE_TEST=tone` publishes a 440 Hz tone; `=listen` logs received levels and gains |

End-to-end run (logs): a second client subscribed to the first's track; gain fell from 1.00 near to 0.40 at 12 tiles and 0.00 at 16 tiles; unsubscribed near 18 and resubscribed inside 16; the gain was applied to the real audio. Wrong secret and unreachable server each gave one warning and the game continued.

Never verified: a real microphone, echo cancellation in practice, the macOS mic-permission prompt, push-to-talk with a held key, the online settings panel, and owner mute against a live LiveKit server. Both open questions are decided (V-D6, V-D7 below) and built.

## Production (reported by the deployment session, 2026-11-13)

- LiveKit `v1.13.11` in Docker on the production host, behind nginx with TLS at `wss://livekit.kelasmalam.app`
  (Cloudflare-proxied signalling); media on 50000–60000/udp and 7881/tcp, TURN on 3478/udp. LiveKit's connection
  test passed (signalling, WebRTC, TURN, resume).
- The deployed game server gets `LIVEKIT_URL` / `LIVEKIT_API_KEY` / `LIVEKIT_API_SECRET` from the server's `.env`
  (passed through `infra/game/docker-compose.yml`); empty values keep voice off.
- Secret-free template of the production setup: `infra/livekit/prod/` (`docker-compose.yml`, `livekit.yaml` with a
  placeholder key, `nginx.conf`). Real keys live only on the server (`/opt/livekit/livekit.yaml`, the game `.env`;
  `.env` is git-ignored).
- The game server's LiveKit API calls go through the same public URL; LiveKit itself answers there (an unsigned call
  returns LiveKit's own 401), so signed calls get through.
- Not yet tested in production: two clients with real microphones, owner mute, and removal on leave.

## Decisions (owner, relayed by the asset session, 2026-11-13)

| # | Decision |
|---|---|
| V-D1 | **Self-hosted** LiveKit, using `infra/livekit` |
| V-D2 | **Proximity** voice: loud when near, quiet then silent when far (the curve above) |
| V-D3 | **Both** push-to-talk (hold **V**) and open mic, chosen in a voice settings menu. Voice stays **off** until the player turns it on |
| V-D4 | Built by the research/QC session's scripting subagent. The game server and `net.rs` belong to the coding session, so the token hook-up is agreed with it first |
| V-D5 | Age gating, reporting, recording policy: still open, out of scope |
| V-D6 | Push-to-talk opens the OS microphone **only while V is held** (open mic keeps it open). Owner, 2026-11-13 |
| V-D7 | **Staff (moderators and admins) may voice-mute** as well as the world claim owner, never someone of equal or higher role. Owner, 2026-11-13 |
