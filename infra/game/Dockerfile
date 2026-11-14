# world_server image. Build context is the repo root: docker compose -f infra/game/docker-compose.yml build
FROM rust:1.97-bookworm AS build
WORKDIR /src
COPY . .
# One job by default so the build does not starve other services on the small shared VPS;
# `--build-arg JOBS=2` when a deploy is urgent.
ARG JOBS=1
RUN cargo build --release -p world_server -j ${JOBS}

FROM debian:bookworm-slim
COPY --from=build /src/target/release/world_server /usr/local/bin/world_server
WORKDIR /data
EXPOSE 7777/udp
CMD ["world_server"]
