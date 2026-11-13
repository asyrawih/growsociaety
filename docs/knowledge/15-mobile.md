# 15 · Mobile (iOS and Android)

Plan, not built. Written 2026-11-14 from a read of the current client code and research into Bevy, LiveKit and the
app stores. The client is one Rust/Bevy crate, so mobile is the same game with platform fixes, touch controls and
store work, not a rewrite.

## Knowledge

### What carries over unchanged

| Part | Why it works on mobile |
|---|---|
| `game_shared` (rules, physics, items, farming, claims, trade, levels) | Pure Rust, no IO, no Bevy |
| Networking (`quinn` QUIC, `postcard`) | Plain UDP sockets, supported on iOS and Android |
| Bevy rendering, sprites, UI, 9-slice kit, pixel font | Bevy targets iOS (`aarch64-apple-ios`, simulator) and Android (`aarch64-linux-android`) |
| Voice (LiveKit Rust SDK) | The SDK lists iOS and Android as supported; its WebRTC build downloads prebuilt libraries per target (iOS device and simulator; Android uses the NDK). To verify in the spike |
| Audio capture and playback (`cpal`) | CoreAudio on iOS, AAudio on Android |

### What breaks today (found in the code, 2026-11-14)

| Problem | Where | Why it breaks | Fix |
|---|---|---|---|
| **Lobby re-executes the process** to hand the token to the game | `lobby.rs` (`Command::new(current_exe())`) | iOS and Android apps cannot re-exec themselves | One App with a state machine: `Lobby → InGame` in-process, token passed in memory |
| **Files read straight from disk** | `player.rs`, `wardrobe.rs`, `icon.rs`, `main.rs` (`FileAssetReader::get_base_path()`, `std::fs::read`) | On Android, assets live inside the APK, not on a filesystem | Load every image through Bevy's `AssetServer` (it has an Android APK reader), or embed small files with `include_bytes!` |
| **Settings path** from `HOME` / `XDG` / `APPDATA` | `settings.rs::config_dir()` | Neither variable points to the app's private storage on mobile | iOS: the app container's `Library/Application Support`; Android: the app's internal data path from the Android app handle |
| **Production server is an IPv4 literal** | `net.rs` `PROD_SERVER = "76.13.196.13:7777"` | Apple requires apps to work on IPv6-only (NAT64) networks and App Review tests there; an IPv4 literal fails | A DNS name (e.g. `game.<domain>`) with A and AAAA records; resolve at connect time |
| **Pinned certificate read from a file path** | `net.rs` (`world_server_cert.der` from the working directory) | No working directory on mobile | Bundle the certificate (or its hash) into the binary with `include_bytes!`, or move to a real CA certificate on the DNS name |
| **Keyboard-only actions** | `interact.rs`, `hud.rs`, `settings.rs`, `chat.rs`, voice (A/D/W/Space, Q, X, E/I, B, N, V, Enter, 1–8, wheel zoom) | No keyboard | Touch controls (below) |
| **Mouse wheel zoom** | `camera.rs` | No wheel | Pinch to zoom, plus the existing default-zoom setting |

### Touch controls (design)

Landscape only. Both thumbs on the screen edges, the world in the middle.

| Need | Control |
|---|---|
| Walk | Left-thumb pad: a left and a right button (a full joystick is overkill: movement is one axis) |
| Jump | Right-thumb button; hold for a full jump, tap for a hop (physics already supports both) |
| Punch / place | Tap a tile; hold on it to repeat at the punch cadence (doc 05 already maps tap to the left-click path) |
| Select item | Tap a hotbar slot (exists) |
| Drop / trash | Buttons in the inventory and wardrobe panels on the selected item (trash keeps its two-tap confirm) |
| Inventory, wardrobe, store, settings, chat | The existing top bar |
| Push-to-talk | A hold button above the jump button while voice is on in push-to-talk mode |
| Zoom | Pinch; the Display setting picks the default |
| Chat | The top-bar chat button opens the line and the on-screen keyboard |

Rules: every touch target at least 44 pt; nothing interactive inside the notch, home-indicator or system-gesture
areas (safe areas); the HUD scales with the device's scale factor; touches on any UI node never reach the world (the
existing "pointer over UI" check, extended to touch).

Known risk: showing the on-screen keyboard and text input (IME) from a Bevy text field is the least mature part of
mobile support in the windowing layer. The spike must test it early; the fallback is a small native text field
bridge per platform.

### App lifecycle and network

- When the app goes to the background, the OS may suspend it and drop the QUIC connection. On resume: reconnect with
  the stored token if it is still valid (tokens last 15 minutes), otherwise return to the lobby. The game must not
  crash or hang on a dead connection.
- Pause rendering and audio in the background; release the microphone. On Android the rendering surface is destroyed
  and recreated; Bevy handles this, but our code must not hold onto window-specific state across it.
- Cap the frame rate at 60 and keep the frame budget small (one sprite per tile is about 5,500 entities; fine on
  current phones, measure on a mid-range Android).

### Permissions

| Permission | iOS | Android |
|---|---|---|
| Network | Allowed by default | `INTERNET` in the manifest |
| Microphone (voice) | `NSMicrophoneUsageDescription` text in `Info.plist`; the system prompt appears on first capture | `RECORD_AUDIO` in the manifest **and** a runtime request before the first capture (needs a small JNI call) |
| Audio while recording | Audio session category for play-and-record, so music and voice coexist | Audio focus handling |

Voice stays off until the player turns it on (doc 12), so the microphone prompt only appears when a player chooses
voice.

### App store requirements that touch the game

These are review blockers, not polish. Verify each against the current guideline text before submitting.

| Requirement | Store | Status in the game |
|---|---|---|
| User-generated content (chat, voice, building) needs: a content **filter**, a way to **report** users and content with timely response, **blocking** of abusive users, published **contact** information | Apple guideline 1.2; Google Play UGC policy (also: users accept terms before creating UGC) | Partial: `/ignore` and per-player voice mute exist; profanity filter is planned (doc 10); report queue is planned (doc 13, C3–C4); terms acceptance and contact info do not exist |
| **Account deletion** inside the app for apps that let users create accounts | Apple 5.1.1(v); Google Play also asks for a deletion path | Missing |
| Privacy policy URL, data-safety / privacy labels | Both | Missing |
| Age rating questionnaire (chat and voice with strangers) | Both | To fill in; expect a teen rating |
| Selling digital goods for real money must use the store's in-app purchase | Both | Not applicable yet (gems cannot be bought); becomes required if they ever are |
| Works on IPv6-only networks | Apple | Blocked by the IPv4 literal above |

### Versions in the wild

Store updates reach players slowly (review time, people not updating). The server must either accept the previous
`PROTOCOL_VERSION` for a while or answer an outdated client with a clear "please update" message instead of a bare
refusal. Desktop gets away with lockstep versions; mobile does not.

## Technical

### Build tooling

| Platform | Tooling | Notes |
|---|---|---|
| iOS | Rust target `aarch64-apple-ios` (+ `aarch64-apple-ios-sim`), an Xcode project that links the Rust static library | Bevy ships a mobile example with an Xcode project; `cargo-mobile2` can generate one. Needs macOS + Xcode, an Apple Developer account for devices and TestFlight |
| Android | `cargo-ndk` to build `.so` libraries per ABI, a small Gradle project with **GameActivity** (Bevy's default; requires Android API 31+) | `NativeActivity` is the alternative for older devices. Needs the Android SDK/NDK (`ANDROID_NDK_HOME`, also used by the WebRTC build) and a Play Console account |

The crate becomes a library plus a thin desktop binary: mobile entry points call the same `App` setup. Follow the
Bevy 0.20 mobile example at the matching git tag; mobile setup details change between Bevy releases, so do not copy
older guides.

Cargo features: keep `voice` optional so the first mobile builds can skip WebRTC (`--no-default-features`).

### Phases

| Phase | Delivers | Can start now? |
|---|---|---|
| P0 Spike | A blank Bevy 0.20 app on an iOS simulator, an iPhone and an Android phone; then our client built for both with `--no-default-features`; then with `voice`. Test the on-screen keyboard early | Yes |
| P1 Portability (also good for desktop) | In-process lobby (no re-exec); all images through `AssetServer`; per-platform settings directory; DNS server name with IPv6; bundled certificate; crate split into lib + desktop bin | Yes, but it touches files owned by other sessions (lobby, net, player, wardrobe, main) |
| P2 Touch controls and layout | The controls above, safe areas, scale factor, pinch zoom, landscape lock, touch-aware "pointer over UI" | After P0 |
| P3 Voice on mobile | Microphone permission flow on both, audio session/focus, echo cancellation tested on device speakers | After P0 with `voice` building |
| P4 Lifecycle | Background/resume, reconnect with token, release mic, pause audio | After P1 |
| P5 Store readiness | Account deletion, report/block/filter complete, terms acceptance, privacy policy, contact info, icons and splash, signing, TestFlight and Play internal testing | Parallel to P2–P4; several items are server-side |
| P6 Version policy | Server accepts the previous protocol version or sends "please update"; designed in [16](16-auto-update.md) | Before the first public release |

### Tests and acceptance

- Unit: touch-control mapping to `Input` (left/right/jump), hold-to-repeat timing, pinch zoom level mapping, safe-area
  layout math, settings directory per platform (cfg-gated), protocol "please update" path.
- Device checks per phase (owner or a tester): the game runs at 60 fps on a mid-range Android and a recent iPhone;
  walk, jump, punch, place, inventory, wardrobe, store, chat with the on-screen keyboard; two phones in one world see
  each other; voice between two phones with real microphones and no echo on speaker; background for 1 minute and
  resume; airplane mode on and off; an IPv6-only network (Apple's NAT64 test setup).

## Open decisions for the owner

| # | Question | Default in this plan |
|---|---|---|
| MO-D1 | Minimum OS versions | iOS 16+; Android API 31+ (GameActivity) |
| MO-D2 | Orientation | Landscape only |
| MO-D3 | Movement controls | Two buttons (left/right) + jump, not a joystick |
| MO-D4 | Voice on mobile at first release, or desktop-only voice first? | Included, after P3 passes on devices |
| MO-D5 | Store accounts (Apple Developer, Google Play Console) and who owns them | Owner's accounts |
| MO-D6 | Domain name for the game server (needed for IPv6 and a real certificate) | A subdomain of the existing domain |
| MO-D7 | Who builds which phase | P1 needs the coding session (lobby, net) and growsociaety-75 (lobby); P2 the research/QC session's scripting subagent (HUD, input); P5 split by owner of each system |
