#!/usr/bin/env bash
# Deploys world_server to prod without compiling there: cross-compile on this Mac, copy the binary
# next to the compose file on the server, rebuild only the `world` container (never `db`).
#   scripts/deploy-game.sh            # needs the owner's OK for prod
# Needs: zig, cargo-zigbuild, `rustup target add x86_64-unknown-linux-gnu`.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST=${GAME_HOST:-root@76.13.196.13}
DIR=${GAME_DIR:-/opt/growsociety}      # repo copy on the server; compose lives in infra/game
TARGET=x86_64-unknown-linux-gnu
GLIBC=2.36                             # debian:bookworm-slim, the runtime image

# Server-only profile: no LTO, parallel codegen. The client keeps the workspace release profile.
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    cargo zigbuild --release -p world_server --target "$TARGET.$GLIBC"
mkdir -p infra/game/bin
cp "target/$TARGET/release/world_server" infra/game/bin/world_server
file infra/game/bin/world_server | grep -q "ELF 64-bit.*x86-64" || {
    echo "not an x86_64 Linux binary" >&2
    exit 1
}

# Only what the image and compose need; .env and the /data volume (cert, keys) stay untouched.
rsync -az .dockerignore "$HOST:$DIR/"
rsync -az infra/game/Dockerfile infra/game/docker-compose.yml "$HOST:$DIR/infra/game/"
rsync -az infra/game/bin/world_server "$HOST:$DIR/infra/game/bin/"
ssh "$HOST" "cd $DIR/infra/game && docker compose up -d --build world && sleep 5 && docker compose logs world --since 30s | tail -5"
