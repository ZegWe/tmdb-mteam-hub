# syntax=docker/dockerfile:1.7

ARG NODE_VERSION=22.18.0
ARG RUST_VERSION=1.96.0
ARG OCI_SOURCE=https://github.com/ZegWe/tmdb-mteam-hub
ARG OCI_REVISION=unknown
ARG OCI_VERSION=dev
ARG OCI_CREATED=1970-01-01T00:00:00Z

FROM node:${NODE_VERSION}-bookworm-slim AS frontend
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY frontend ./frontend
COPY vite.config.ts tailwind.config.js postcss.config.cjs ./
RUN npm run build

FROM rust:${RUST_VERSION}-bookworm AS backend
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --locked

FROM debian:bookworm-slim AS runtime
ARG OCI_SOURCE
ARG OCI_REVISION
ARG OCI_VERSION
ARG OCI_CREATED

WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data/config /data/state /data/cache/tmdb /data/cache/douban /srv/media
COPY --from=backend /app/target/release/tmdb-mteam-server /usr/local/bin/tmdb-mteam-server
COPY --from=frontend /app/static ./static

LABEL org.opencontainers.image.source="${OCI_SOURCE}" \
      org.opencontainers.image.revision="${OCI_REVISION}" \
      org.opencontainers.image.version="${OCI_VERSION}" \
      org.opencontainers.image.created="${OCI_CREATED}"

ENV CONFIG_PATH=/data/config/config.toml \
    TMDB_CACHE_DIR=/data/cache/tmdb \
    DOUBAN_CACHE_DIR=/data/cache/douban \
    SUBSCRIPTION_STATE_DIR=/data/state \
    HEALTHCHECK_URL=http://127.0.0.1:8787/healthz \
    RUST_LOG=tmdb_mteam_server=info,tower_http=info

VOLUME ["/data/config", "/data/state", "/data/cache/tmdb", "/data/cache/douban", "/srv/media"]
EXPOSE 8787

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error --output /dev/null "$HEALTHCHECK_URL" || exit 1

CMD ["tmdb-mteam-server"]
