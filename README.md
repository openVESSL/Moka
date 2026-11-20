# Moka

Self-hosted media library manager. One program for films, shows and music: browse a catalog, request a title, let it be fetched through the download client and indexers you configure, imported into your library, and played in the browser.

Moka ships with no content sources. You connect your own download clients and indexers.

> **Development preview.** Moka is under active development and not ready for
> everyday use. There are no releases yet, settings and the database may change
> between commits, and things will break. Try it on a copy of your media, not
> the only one, and please report what you find.

## Status

Working in development builds, not yet tested widely:

- Films and shows: catalog, requests with approval, automatic release selection with quality profiles, download tracking (qBittorrent, Transmission, SABnzbd), import with hardlinks and renaming, existing folders brought in as they are.
- Shows: episodes or whole seasons, new episodes followed as they air.
- In-browser player: direct play, remux or conversion with FFmpeg (optionally on a graphics card), subtitles inside or next to the file.
- Works next to other self-hosted media apps through their usual APIs and webhooks.
- Music: catalog and album requests. Downloading and playing music are in progress.

Planned: ebooks and audiobooks, an API that existing phone and TV apps can connect to, a new interface.

## Stack

Rust (Axum, Tokio, SQLx) backend, React frontend, SQLite by default. Ships as a single binary and a Docker image.

## Development

Requirements: Rust (stable), Node.js 22+ with pnpm.

```sh
# web app (rebuild before building the server, it is embedded in the binary)
pnpm --dir web install
pnpm --dir web build

# server on http://localhost:8484
cargo run -p moka-server

# web dev server with hot reload on http://localhost:5173 (proxies /api to :8484)
pnpm --dir web dev
```

Checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir web build
```

After changing the HTTP API, regenerate the web client types:

```sh
pnpm --dir web gen:api
```

Configuration is read from environment variables:

| Variable | Default | |
|---|---|---|
| `MOKA_CONFIG_DIR` | `./config` | database, caches, logs |
| `MOKA_PORT` | `8484` | |
| `MOKA_HOST` | `0.0.0.0` | |
| `MOKA_DATABASE_URL` | SQLite in the config dir | |
| `MOKA_SECRET_KEY` | `secret.key` in the config dir | base64, 32 bytes; encrypts stored passwords and API keys |

Back up `secret.key` together with the database: without it, stored indexer keys and passwords cannot be read and must be entered again.

See [docs/architecture.md](docs/architecture.md) for how the pieces fit together, and
[docker-compose.example.yml](docker-compose.example.yml) for a container setup.

## License

Copyright (C) 2026 OpenVessl and contributors.

Moka is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License, version 3 (GPL-3.0-only).
See [LICENSE](LICENSE).

This product uses the TMDB API but is not endorsed or certified by TMDB. Music metadata comes from MusicBrainz and album covers from the Cover Art Archive.
