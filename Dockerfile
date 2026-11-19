# syntax=docker/dockerfile:1

# --- web app ---
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# --- server (embeds the web app) ---
FROM rust:1-bookworm AS server
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates crates
COPY --from=web /src/web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p koda-server && cp target/release/koda /koda

# --- runtime ---
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home koda \
    && mkdir -p /config /data && chown koda:koda /config /data
COPY --from=server /koda /usr/local/bin/koda

ENV KODA_CONFIG_DIR=/config \
    KODA_PORT=8484 \
    RUST_LOG=info,sqlx=warn
VOLUME ["/config"]
EXPOSE 8484
USER koda
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/koda"]
