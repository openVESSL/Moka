# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). While the version is 0.x, a minor
release may change behavior or configuration; such changes are listed with
the steps to migrate.

## [Unreleased]

### Added
- Jellyfin alongside Koda: targeted library refresh after each import (with
  optional path mapping) and watched status synced back for matching user names.
- Import of indexers from Prowlarr, torrent and Usenet, searched through Prowlarr.
- Transmission and SABnzbd download clients; Usenet releases from Newznab
  indexers are sent to SABnzbd and tracked to import.
- API keys for other apps, sent as `X-Api-Key` or `?apikey=`.
- A `/api/v3` API compatible with Radarr: indexer managers can sync
  Torznab/Newznab indexers into Koda, and request apps can add films,
  follow their status and see the download queue.
- Subtitle apps that work with Radarr can read films with their file paths
  and release names from `/api/v3`, and webhooks pointed at their
  `/api/webhooks/radarr` address are told about each import.
- Koda in front of an existing Radarr: approved films are sent to Radarr,
  followed until they have a file, and Radarr's downloaded films are listed
  in Koda's library (with optional path mapping).
- Subtitles in the player: tracks inside the file and subtitle files next
  to it, shown as WebVTT and kept in step when seeking; the chosen language
  is remembered. ASS/SSA subtitles keep their styling (rendered in the
  browser), with the fonts attached to MKV files. Image subtitles (PGS, VobSub) are drawn into the picture, which
  converts the film while they are on.
- Video conversion on a graphics card (NVENC, VAAPI, Quick Sync), with a
  fallback to the processor if the card fails.

### Fixed
- CI reads the pnpm version from `web/package.json`.
- Web install no longer fails on a dependency's install script, which is
  now skipped (it only downloads a font list).

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
