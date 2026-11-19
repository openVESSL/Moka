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

## License

GPL-3.0. See [LICENSE](LICENSE).

This product uses the TMDB API but is not endorsed or certified by TMDB.
