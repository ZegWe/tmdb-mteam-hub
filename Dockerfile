# syntax=docker/dockerfile:1.7

FROM node:22-bookworm-slim AS frontend
WORKDIR /app
COPY package.json package-lock.json vite.config.ts tailwind.config.js postcss.config.cjs ./
COPY frontend ./frontend
RUN npm ci
RUN npm run build

FROM rust:1-bookworm AS backend
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY --from=frontend /app/static ./static
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data/config /data/cache
COPY --from=backend /app/target/release/tmdb-mteam-server /usr/local/bin/tmdb-mteam-server
COPY --from=backend /app/static ./static

ENV CONFIG_PATH=/data/config/config.toml \
    TMDB_CACHE_DIR=/data/cache/tmdb \
    DOUBAN_CACHE_DIR=/data/cache/douban \
    SUBSCRIPTION_STATE_DIR=/data/cache/subscriptions \
    RUST_LOG=tmdb_mteam_server=info,tower_http=info

VOLUME ["/data/config", "/data/cache"]
EXPOSE 8787

CMD ["tmdb-mteam-server"]
