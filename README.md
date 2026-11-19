# Koda

Self-hosted media library manager. One program for your whole movie and TV setup: browse a catalog, request a title, let it be fetched through the download client and indexers you configure, imported into your library, and played in the browser.

Koda ships with no content sources. You connect your own download clients and indexers.

> Early development. Nothing to install yet.

## Planned

- Catalog search and requests (TMDB metadata)
- Connectors: download clients and Torznab/Newznab indexers
- Automatic release selection with quality profiles
- Import with hardlink/move and renaming
- Library browser and in-browser player (direct play, then transcoding via FFmpeg)
- Multi-user with permissions and request approval

## Stack

Rust (Axum, Tokio, SQLx) backend, React frontend, SQLite by default. Ships as a single binary and a Docker image.

## Development

Requirements: Rust (stable), Node.js 22+ with pnpm.

```sh
# web app (rebuild before building the server, it is embedded in the binary)
pnpm --dir web install
pnpm --dir web build

# server on http://localhost:8484
cargo run -p koda-server

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
| `KODA_CONFIG_DIR` | `./config` | database, caches, logs |
| `KODA_PORT` | `8484` | |
| `KODA_HOST` | `0.0.0.0` | |
| `KODA_DATABASE_URL` | SQLite in the config dir | |

See [docs/architecture.md](docs/architecture.md) for how the pieces fit together, and
[docker-compose.example.yml](docker-compose.example.yml) for a container setup.

## License

GPL-3.0. See [LICENSE](LICENSE).

This product uses the TMDB API but is not endorsed or certified by TMDB.
