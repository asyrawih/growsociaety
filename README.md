# Rust 2D Game Engine

A 2D multiplayer sandbox game built with Rust and Bevy, inspired by tile-based sandbox MMO gameplay.

Design and technical docs: [`docs/knowledge/`](docs/knowledge/README.md).

## Run

```sh
cargo run -p game_client
cargo test --workspace
```

Day-to-day client work: `cargo dev` (no voice, so no LiveKit/WebRTC build, and Bevy is linked dynamically for fast
rebuilds). Use `cargo devv` only when working on voice. Release builds (`cargo build --release`, CI) never enable either.

Online (authoritative server, QUIC on UDP 7777):

```sh
cargo run -p world_server                      # writes world_server_cert.der
GAME_SERVER=localhost cargo run -p game_client # same directory, so it finds the cert
```

Optional: `GAME_WORLD=NAME`, `GAME_TOKEN=...`, `GAME_SERVER=host:port`, `GAME_SERVER_CERT=path`.

## Current workspace

```text
client/game_client/   Bevy client (binary); online mode in src/net.rs
crates/game_shared/   engine-free shared rules (world size, tile coords)
crates/game_protocol/ wire messages + framing (postcard over QUIC)
server/world_server/  authoritative server: one Tokio task per world
```

More crates get added when their roadmap phase starts (e.g. `server/world_server` + `game_protocol` in Phase 5),
rather than as empty stubs.

## CI / Release

`.github/workflows/ci.yml`:
- push/PR: `fmt --check`, `clippy -D warnings`, `test` on Linux.
- tag `v*`: release build for Linux, Windows, macOS, zipped and attached to a GitHub Release.

```sh
git tag v0.1.0 && git push --tags
```

## Technology Stack

| Component | Technology |
|---|---|
| Language | Rust |
| Client Engine | Bevy 0.20 |
| Server Runtime | Tokio |
| Networking | UDP / Custom Protocol |
| Serialization | Bincode / MessagePack |
| Database | PostgreSQL |
| Cache | Redis |
| Physics | Custom AABB |
| World Storage | Chunk-based Binary |
| Build System | Cargo Workspace |

## Initial Vertical Slice

1. Render a 100 × 50 tile world.
2. Render a player character using a sprite atlas.
3. Support horizontal movement.
4. Implement gravity and jumping.
5. Implement AABB tile collision.
6. Implement smooth camera follow.
7. Support breaking blocks.
8. Support placing blocks.

## Roadmap

- Phase 1 — Engine Foundation: [x] Cargo workspace, [x] CI pipeline, [ ] plugin architecture, [ ] configuration
- Phase 2 — Rendering: sprite renderer, texture atlas, render ordering, tilemap renderer, camera
- Phase 3 — Character: entity, movement controller, gravity, AABB collision, animation state machine
- Phase 4 — World: tile definitions, chunks, generation, block destroy/place
- Phase 5 — Multiplayer: protocol, connection, auth, player/world sync, server authority
- Phase 6 — Gameplay: inventory, items, equipment, trading, world ownership, economy
- Phase 7 — Persistence: player/world DB, inventory persistence, transaction safety, recovery

## Engineering Principles

1. Modular architecture.
2. Server-authoritative multiplayer.
3. Shared gameplay definitions (`game_shared` stays engine-free).
4. Data-driven items and tiles.
5. Efficient tile and chunk rendering.
6. Minimal unnecessary allocations.
7. Deterministic gameplay rules where practical.
8. Performance-first architecture.
9. Mobile-friendly rendering and input.
10. Independent client and server lifecycles.

## Configuration Decisions

| Decision | Status |
|---|---|
| Bevy version | 0.20.1 (confirmed) |
| Target platform | Desktop (Linux/Windows/macOS) in CI; mobile to confirm |
| Rendering style | To confirm |
| Multiplayer capacity | To confirm |
| Network transport | To validate |
| World chunk dimensions | To define |
