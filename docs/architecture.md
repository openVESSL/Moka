# Architecture

Status: draft, describes the target for the first release (MVP) and the direction after it.

## Goals

- One self-hosted program that covers the whole path: browse a catalog, request a title, find a release, download it through the user's own client, import it into a library, play it in the browser.
- Starts empty. Ships with no content sources. The user adds their own download client and indexers.
- Runs well on small hardware (NAS, Raspberry Pi): one binary, low memory, SQLite by default.
- Every automatic decision is explainable in the UI (why a release was rejected, why a stream is transcoded).

## Non-goals (for now)

- Distributing content or bundling indexers.
- Store apps with download features. Store apps are playback-only clients.
- A plugin system. Features live in the main codebase until there is a real need.

---

## 1. Big picture

```
                      ┌──────────────────────── koda (single binary) ────────────────────────┐
 Browser (React SPA) ─┤ HTTP API /api/v1  ·  SSE /api/v1/events  ·  static web assets         │
 Store/TV apps ───────┤ Compatibility API (later)                                             │
                      │                                                                       │
                      │  requests ─▶ titles (state machine) ◀─ reconciler (background loop)   │
                      │                 │                          │                          │
                      │   metadata ◀────┘          indexers ◀──────┤                          │
                      │   (TMDB)                   download ◀──────┤                          │
                      │                            importer ◀──────┤                          │
                      │                            library  ◀──────┘                          │
                      │                            streaming (ffmpeg subprocess)              │
                      │                                                                       │
                      │                     SQLite (or Postgres)                              │
                      └───────────────────────────────────────────────────────────────────────┘
                               │                  │                     │
                           TMDB API        Torznab/Newznab        qBittorrent Web API
```

A modular monolith. Modules are crates in one Cargo workspace. They share one database and one data model. There is no sync between components because there is only one component.

---

## 2. Data model

### 2.1 Title

The central object. A movie, a show, a season or an episode. The MVP only uses `movie`.

| Field | Notes |
|---|---|
| `id` | integer primary key |
| `kind` | `movie` (later `show`, `season`, `episode`) |
| `parent_id` | season → show, episode → season (later) |
| `tmdb_id`, `imdb_id` | external ids. `(kind, tmdb_id)` is unique |
| `title`, `original_title`, `year`, `overview`, `runtime_minutes` | cached metadata |
| `release_date`, `digital_release_date`, `physical_release_date` | used for availability |
| `poster_path`, `backdrop_path`, `logo_path` | TMDB image paths, served through the image proxy |
| `metadata_json` | full cached TMDB payload (credits, genres, videos) |
| `state` | see 2.2 |
| `state_reason` | short human text for the current state ("no release matched the profile") |
| `quality_profile_id` | which profile drives release selection |
| `monitored` | if false, the reconciler leaves the title alone |
| `next_search_at`, `last_search_at` | search scheduling |
| `created_at`, `updated_at` | |

### 2.2 Title states

```
 requested ──approve──▶ wanted ──▶ searching ──found──▶ grabbed ──▶ downloading ──▶ importing ──▶ available
     │                    ▲            │                   │            │              │
  decline                 │        none found              └── failed ──┴── failed ────┘
     ▼                    └──── back to wanted, retry later (with backoff) ◀───────────┘
  (request closed,
   title removed if
   nothing else uses it)
```

- `requested`: a user asked for it and it waits for approval. Titles requested by a user with auto-approve skip straight to `wanted`.
- `wanted`: approved, no file yet. The reconciler will search when `next_search_at` passes.
- `searching`: an indexer search is running.
- `grabbed`: a release was chosen and sent to the download client. A `grab` row exists.
- `downloading`: the client reports progress.
- `importing`: the download finished. Files are being linked, copied or moved into the library.
- `available`: a media file is in the library and can be played.

Failures never leave a title stuck. A failed grab or import is recorded on the `grab` row, the release goes on the blocklist if it was bad, and the title goes back to `wanted` with a `state_reason`.

`watched` is **not** a title state. It is per user, stored in `user_title_data`. One person watching a movie does not change it for anyone else.

Upgrades (a better release for an already available title) come later. The title stays `available` and a second grab runs alongside.

### 2.3 Other tables (MVP)

| Table | Purpose |
|---|---|
| `users` | username, password hash (argon2id), permissions bitmask, quota overrides |
| `sessions` | web login sessions (opaque token, expiry) |
| `api_keys` | tokens for integrations (bots, scripts), hashed |
| `requests` | who asked for which title, status (`pending`, `approved`, `declined`), decided by/at, note. Many users can request the same title |
| `quality_profiles` | ordered list of allowed qualities, cutoff, min/max size rules |
| `indexers` | Torznab/Newznab endpoints: url, api path, api key, categories, priority, enabled, cached capabilities |
| `indexer_status` | backoff level, disabled until, last error |
| `download_clients` | qBittorrent connection, category, enabled |
| `grabs` | one row per release sent to a client: title, indexer, release title, size, quality, `download_client_id`, `info_hash`, status, error |
| `blocklist` | releases that failed or were rejected after download (by info hash and title) |
| `media_files` | file in the library: title, path, size, quality, probed streams (json), added at |
| `user_title_data` | per user: playback position, played, play count, last played, favorite |
| `events` | append-only history (requested, approved, grabbed, imported, played...) used for the activity feed and notifications |
| `settings` | key/value for global settings (library paths, TMDB language/region, naming template) |
| `webhooks` | outgoing notification targets |

Secrets (indexer API keys, download client passwords) are stored in the database. The API never returns them: they are write-only fields. Logs redact them, and URLs with keys are never logged in full.

### 2.4 Quality

A quality is a combination of:

- **source**: `cam`, `telesync`, `telecine`, `workprint`, `dvd`, `tv`, `webrip`, `webdl`, `bluray`
- **resolution**: `480`, `576`, `720`, `1080`, `2160` (or unknown)
- **modifier**: `none`, `remux`, `disc` (full disc image, not playable directly), `screener`, `regional`, `raw`

Plus a **revision**: version (repack/proper count) and a `real` flag. Order between qualities is not built in: it comes from the quality profile the user edits. A default profile ships with sensible ordering.

Size limits are expressed per minute of runtime (MB/min), so one rule fits short and long movies.

---

## 3. Modules (crates)

| Crate | Responsibility | MVP |
|---|---|---|
| `koda-core` | shared types: ids, `TitleState`, quality model, errors, config | yes |
| `koda-db` | SQLx pool, migrations, queries | yes |
| `koda-parser` | release name parsing: title, year, quality, group, edition, revision, languages | yes |
| `koda-metadata` | TMDB client, caching, image proxy | yes |
| `koda-indexers` | Torznab/Newznab client: capabilities, search, result normalization, rate limiting, backoff | yes |
| `koda-download` | download client trait + qBittorrent implementation | yes |
| `koda-decision` | rules that accept/reject releases, ranking | yes |
| `koda-importer` | link/copy/move into the library, renaming, sample detection | yes |
| `koda-library` | folder scan for existing libraries, file watcher, ffprobe | yes |
| `koda-streaming` | play method decision, HLS sessions, ffmpeg process management | yes |
| `koda-server` | the binary: config, HTTP API, auth, SSE, reconciler, serves the web app | yes |
| `web/` | React app | yes |

Crate names carry the working project name and will be renamed together with the product.

Rules between crates:

- Leaf crates (`parser`, `metadata`, `indexers`, `download`) know nothing about the database. They take inputs and return plain data, so they can be tested alone.
- Only `koda-db` writes SQL. Only `koda-server` wires things together.
- External HTTP clients take a base URL, so tests run against local mock servers.

---

## 4. Background work: the reconciler

Instead of a web of events and handlers, one loop looks at the database and moves each title one step forward. Every step is idempotent, so a restart at any moment is safe: the loop just picks up where the data says things are.

```
every tick (a few seconds while something is active, ~60 s when idle):
  wanted      and next_search_at <= now    → search (parallel over indexers), decide, grab or reschedule
  grabbed / downloading                    → ask the client for the info_hash, update progress, detect completion/failure
  importing                                → run import, then available (or back to wanted with a reason)
```

- Searches for `wanted` titles back off: 15 min, 1 h, 6 h, then every 12-24 h. A new request triggers an immediate search.
- Progress updates are not written to the database on every poll. They go to the live event stream. Only state changes are persisted.
- Long work (search, import, ffprobe) runs in its own task with a per-title lock, so one slow indexer never blocks the loop.

## 5. Release selection

1. **Search**: query every enabled indexer in parallel (timeout ~25 s each) with `t=movie` and the TMDB/IMDb id when the indexer supports it, otherwise `title year`. Partial results are fine.
2. **Parse** each release name (`koda-parser`).
3. **Match**: id from the indexer if present, otherwise normalized title + year (±1) against the title and its alternative titles.
4. **Rules**: each rule returns accept or reject with a reason and a kind (`permanent` or `temporary`). MVP rules:
   - quality allowed by profile;
   - size within limits for this quality and runtime;
   - minimum seeders;
   - not blocklisted;
   - not a disc image or sample;
   - not already grabbed;
   - enough free disk space.
5. **Rank** accepted releases, first difference wins:
   1. profile quality order, then revision;
   2. preferred-word score (later: custom formats);
   3. indexer priority;
   4. seeders, in log10 buckets, so 50 vs 80 is a tie;
   5. size closest to the preferred size.
6. **Grab** the best one. Store the reasons for the top few candidates so the UI can show why the winner won.

## 6. Download tracking and import

- At grab time the info hash is known: from the indexer, from the magnet link or from the downloaded `.torrent` file. It is saved on the `grab` row. Tracking is a lookup by hash, so release names are never re-parsed to find out what a download belongs to.
- Torrents are added with a dedicated category, so the user can see them in their client.
- **Import mode**:
  - **hardlink** when the download and library folders are on the same filesystem. The torrent keeps seeding and the library file uses no extra space;
  - otherwise **copy** while the torrent still seeds;
  - **move** only when the client says seeding is done.
  - Setup checks whether hardlinks work between the two folders and warns early.
- **Import steps**:
  1. list video files;
  2. drop samples (duration much shorter than the runtime) and extras;
  3. pick the main file;
  4. probe it;
  5. place it at `{library}/{Title} ({Year}) {tmdb-ID}/{Title} ({Year}) {tmdb-ID} - {Quality}.{ext}`;
  6. bring matching external subtitles.
- The TMDB id in the folder name makes later rescans unambiguous.

## 7. Playback

The browser asks `POST /api/v1/titles/{id}/play` with what it can decode (detected with `MediaSource.isTypeSupported` and `MediaCapabilities`). The server answers with one of:

1. **Direct**: the original file over HTTP with range requests. No server work.
2. **Remux**: the video stream is copied, the container becomes fragmented MP4 HLS, and audio is converted only if the browser cannot play it. Low CPU.
3. **Transcode**: the video is re-encoded (H.264 first, hardware acceleration later). High CPU.

The answer always lists the reasons (`video codec hevc not supported`, `audio dts not supported`, `container mkv not supported`) so the player can show them.

- **HLS sessions**:
  - the playlist covers the whole movie from the start, so seeking works anywhere; 4 s segments;
  - ffmpeg starts at the requested segment;
  - a seek outside the window already produced restarts ffmpeg at the new point;
  - one ffmpeg process per session, killed after ~60 s without a request, segments in a temp folder with a size cap.
- **Subtitles**:
  - SRT/VTT are sent as WebVTT;
  - ASS/SSA are rendered in the browser (no burn-in);
  - picture subtitles (PGS) are rendered client-side later. Until then they force a transcode, and the UI says so.

## 8. API

- Native API under `/api/v1`, JSON, `snake_case`, documented with OpenAPI generated from the code. The web app uses a TypeScript client generated from that document.
- Live updates: `GET /api/v1/events` (Server-Sent Events). It carries state changes, download progress, request decisions and library changes.
- **Auth**:
  - browser: session cookie (`HttpOnly`, `SameSite=Lax`);
  - integrations: `Authorization: Bearer <api key>`.
  - Permissions are a bitmask: `admin`, `manage_requests`, `request`, `auto_approve`, `manage_library`, `play`.
- **MVP endpoints**:

```
POST   /api/v1/auth/setup                first admin account (only when no users exist)
POST   /api/v1/auth/login | /logout
GET    /api/v1/me

GET    /api/v1/catalog/search?q=          TMDB search
GET    /api/v1/catalog/discover/{list}    trending | popular | upcoming
GET    /api/v1/catalog/movie/{tmdb_id}    details + local state

POST   /api/v1/requests                   { tmdb_id, kind }
GET    /api/v1/requests?status=
POST   /api/v1/requests/{id}/approve | /decline

GET    /api/v1/titles?state=&sort=
GET    /api/v1/titles/{id}
POST   /api/v1/titles/{id}/search         manual search now
GET    /api/v1/titles/{id}/releases       last search results with accept/reject reasons
POST   /api/v1/titles/{id}/grab           { release_id } manual pick
POST   /api/v1/titles/{id}/play           playback decision
PUT    /api/v1/titles/{id}/progress       { position_seconds, played }

GET|POST|PUT|DELETE /api/v1/indexers[/{id}]     + POST /api/v1/indexers/{id}/test
GET|POST|PUT|DELETE /api/v1/download-clients[/{id}]  + /test
GET|PUT             /api/v1/quality-profiles[/{id}]
GET|PUT             /api/v1/settings

GET    /api/v1/images/{size}/{path}       cached TMDB images
GET    /api/v1/stream/{session}/...       direct file, HLS playlists and segments, subtitles
GET    /api/v1/events                     SSE
GET    /api/v1/health
```

- **Compatibility API (after MVP)**: a separate adapter that exposes the routes common media-center client apps already use: login, libraries, items, images, playback info, streaming, progress reporting. Existing TV and phone apps can then connect from day one. It maps onto the native services. Internal ids become stable GUIDs derived from `(kind, id)`.

## 9. Configuration and files

- `KODA_CONFIG_DIR` (default `./config`, `/config` in Docker): `koda.db`, `cache/images`, `cache/metadata`, `transcodes/`, `logs/`.
- `KODA_PORT` (default `8484`), `KODA_DATABASE_URL` (default SQLite in the config dir, `postgres://...` optional).
- Everything else is configured in the UI and stored in the database.
- Recommended Docker layout: one volume `/data` with `/data/downloads` and `/data/media` inside it, so hardlinks work.

## 10. Frontend

- React + Vite + TanStack Router + TanStack Query + Tailwind. shadcn/ui only for settings and forms.
- Library, title page and player are custom and image-first:
  - large backdrops, posters, accent colors extracted from the poster;
  - a dark theme by default;
  - an intentional type scale and our own palette.
- Player: `<video>` + hls.js, JASSUB for ASS subtitles, custom controls.
- The build output is embedded in the binary, so there is a single file to ship.

## 11. MVP scope

**In:**
- First-run setup (admin account, TMDB key, library folder, download folder).
- Connectors: qBittorrent; Torznab and Newznab indexers (search only for Newznab until a Usenet client exists).
- Catalog: discover lists, search, movie page with request button and live state.
- Requests: auto-approve for admins, approve/decline for others, simple quotas.
- Automatic search, selection with the default profile, grab, tracking, import (hardlink/copy/move), renaming.
- Manual search with visible accept/reject reasons and manual grab.
- Library view and title page. Import of an existing movie folder (parse + TMDB match, confirm uncertain matches).
- Browser playback: direct, remux, software transcode. External SRT/VTT, ASS in the browser. Resume position, mark played.
- Live updates via SSE. Outgoing webhook on request / grabbed / available / failed.
- Docker image, Docker Compose example, CI.

**Out (planned next, roughly in order):**
1. Compatibility API for existing TV/phone apps.
2. TV shows (seasons, episodes, season packs).
3. Hardware transcoding, HDR tone mapping, picture subtitles client-side.
4. Upgrades and custom formats; more profile rules.
5. More download clients (Transmission, SABnzbd) and Usenet.
6. Multiple users with per-user language, notifications to chat services, issue reports.
7. Anime numbering, daily shows.
8. Optional Postgres.

## 12. Testing

- **Parser**: table-driven tests from a data file of real-world release names and expected results. It is the most test-heavy crate.
- **Connectors**: recorded responses (XML/JSON fixtures) served by a local mock HTTP server.
- **Decision and ranking**: pure functions, unit tests.
- **Database**: migrations run on an in-memory SQLite in tests.
- **End-to-end** (later): Docker Compose with a real qBittorrent and a fake Torznab feed.
