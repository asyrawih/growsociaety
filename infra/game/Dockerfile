# world_server runtime image. Nothing is compiled here: scripts/deploy-game.sh cross-compiles the
# binary on a Mac (x86_64 Linux, glibc 2.36 = bookworm) into infra/game/bin/. Build context is the
# repo root, filtered by /.dockerignore.
FROM debian:bookworm-slim
COPY infra/game/bin/world_server /usr/local/bin/world_server
WORKDIR /data
EXPOSE 7777/udp
CMD ["world_server"]
