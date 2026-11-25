# syntax=docker/dockerfile:1.7
#
# LibriSync Server image. Build from the REPOSITORY ROOT (the server depends on
# native/rust-core by path):
#
#   docker build -f server/Dockerfile -t librisync-server .
#
# No image is published; build it on (or for) the machine that runs it. Avoid
# cross-building another architecture under QEMU — compiling Rust that way is slow and
# has segfaulted rustc for this project before.

# ---- web UI (architecture-independent output) --------------------------------------
FROM --platform=$BUILDPLATFORM node:22-slim AS web
WORKDIR /src/server/web
COPY server/web/package.json server/web/package-lock.json ./
RUN npm ci
COPY server/web/ ./
RUN npm run build

# ---- server binary ---------------------------------------------------------------
FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY native/rust-core/ native/rust-core/
COPY server/ server/
COPY --from=web /src/server/web/dist server/web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/server/target \
    cd server \
 && cargo build --release --locked \
 && cp target/release/librisync-server /usr/local/bin/librisync-server

# ---- runtime ---------------------------------------------------------------------
FROM debian:trixie-slim
# ffmpeg from Debian (7.1, built with libmp3lame) so security updates come with the
# base image; tini as PID 1; setpriv (util-linux, already present) drops privileges.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg ca-certificates tzdata tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 librisync \
 && useradd --uid 1000 --gid 1000 --home-dir /config --no-create-home --shell /usr/sbin/nologin librisync \
 && mkdir -p /config /audiobooks

COPY --from=build /usr/local/bin/librisync-server /usr/local/bin/librisync-server
COPY server/docker/entrypoint.sh /usr/local/bin/entrypoint.sh

ENV LIBRISYNC_CONFIG_DIR=/config \
    LIBRISYNC_LIBRARY_DIR=/audiobooks \
    LIBRISYNC_PORT=8383 \
    PUID=1000 \
    PGID=1000 \
    UMASK=022

VOLUME ["/config", "/audiobooks"]
EXPOSE 8383

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["librisync-server", "healthcheck"]

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
CMD ["librisync-server"]

LABEL org.opencontainers.image.title="LibriSync Server" \
      org.opencontainers.image.description="Self-hosted sync and download service for purchased audiobooks (Audible, Libro.fm, Chirp)" \
      org.opencontainers.image.source="https://github.com/Promises/LibriSync" \
      org.opencontainers.image.licenses="GPL-3.0"
