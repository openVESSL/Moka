# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). While the version is 0.x, a minor
release may change behavior or configuration; such changes are listed with
the steps to migrate.

## [Unreleased]

## [0.1.0] - 2026-11-20

First tagged version. Films only. Not ready for production: keep your
current setup alongside it.

### Added
- Single server binary with an embedded web app; SQLite database with
  migrations and an automatic copy before schema upgrades.
- First-run admin setup, sign-in with sessions, API keys, people and
  permissions management, per-account login throttling.
- Film catalog from TMDB (your own key) with search, lists and film pages;
  artwork cached locally.
- Requests with approval, per-person limits and notification of everyone who
  asked for the same film.
- Torznab and Newznab indexers, qBittorrent download client, both with
  connection tests.
- Release name parser, quality profile editor, and automatic release choice
  with a written reason for every rejected release and for the winner.
- Background search, download tracking by torrent hash, import into the
  library with hardlinks (copy as fallback) and subtitles alongside.
- Import of films already on disk.
- Playback in the browser: direct, repackaged (remux) or converted
  (transcode) with FFmpeg, seeking anywhere, resume per person.
- Webhooks for requested, approved, declined, downloading, ready and failed;
  Discord addresses receive a formatted message.
- Docker image and CI.

### Security
- Stored credentials encrypted at rest; never returned by the API.
- Outgoing requests refuse cloud-metadata and link-local addresses.
- Security headers (CSP, frame and content-type protections).
- Tests that every route requires sign-in and that secrets do not leak.
