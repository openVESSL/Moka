# Architecture

Status: describes the code as it is (films, shows, music) and the direction after it.

## Goals

- One self-hosted program that covers the whole path: browse a catalog, request a title, find a release, download it through the user's own client, import it into a library, play it in the browser.
- Starts empty. Ships with no content sources. The user adds their own download client and indexers.
- Works next to the apps people already run, and can be left at any time: the library is plain folders with readable names.
- Runs well on small hardware (NAS, Raspberry Pi): one binary, low memory, SQLite by default.
- Every automatic decision is explainable in the UI (why a release was rejected, why a stream is converted).

## Non-goals (for now)

- Distributing content or bundling indexers.
- Its own store apps. Phone and TV apps connect through a compatible API instead.
- A plugin system. Features live in the main codebase until there is a real need.

---

## 1. Big picture

```
                      ┌──────────────────────────── moka (single binary) ────────────────────────────┐
 Browser (React SPA) ─┤ /api/v1 (native, OpenAPI) · SSE /api/v1/events · embedded web assets         │
 Other apps ──────────┤ /api/v3 compatibility layer (indexer managers, request and subtitle apps)     │
                      │                                                                               │
                      │  requests ─▶ titles (state machine) ◀─ background loop (pipeline + series)    │
                      │                 │                        │                                    │
                      │   catalogs ◀────┘          indexers ◀────┤                                    │
                      │   (TMDB, MusicBrainz)      download ◀────┤                                    │
                      │                            import   ◀────┘                                    │
                      │                            playback (FFmpeg subprocess, HLS)                  │
                      │                                                                               │
                      │                     SQLite                                                    │
                      └───────────────────────────────────────────────────────────────────────────────┘
                          │             │                │                       │
                      TMDB API     MusicBrainz     Torznab/Newznab     qBittorrent · Transmission · SABnzbd
```

A modular monolith: crates in one Cargo workspace, one database, one data model. There is no sync between components because there is only one component.

---

## 2. Data model

### 2.1 Title

The central object, one table for everything that can be requested, downloaded or played.

| `kind` | `parent_id` | Notes |
|---|---|---|
| `movie` | none | the unit of search and import |
| `show` | none | its state says requested/wanted; the UI derives the rest from its episodes |
| `season` | show | the unit of a season-pack grab |
| `episode` | season | the unit of search and import for shows |
| `artist` | none | created with its first album |
| `album` | artist | the unit of search and import for music |
| `track` | album | disc in `season_number`, position in `episode_number` |

| Field | Notes |
|---|---|
| `tmdb_id`, `imdb_id`, `tvdb_id` | external ids for films and shows. `(kind, tmdb_id)` is unique |
| `mbid` | MusicBrainz id for music. `(kind, mbid)` is unique |
| `season_number`, `episode_number` | episodes and tracks |
| `title`, `original_title`, `year`, `overview`, `runtime_minutes`, `release_date` | cached metadata; `release_date` is the air date for episodes |
| `poster_path`, `backdrop_path` | TMDB image paths, served through the image proxy |
| `metadata_json` | extra catalog data (for shows: how releases are numbered: `standard`, `daily`, `anime`) |
| `state`, `state_reason` | see 2.2; the reason is short plain text shown in the UI |
| `quality_profile_id` | which profile drives release selection |
| `monitored` | if false, the background loop leaves the title alone (specials, episodes missing from an imported library) |
| `next_search_at`, `last_search_at`, `search_attempts` | search scheduling with backoff |

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

- Moves are compare-and-set on the current state, so two workers never move one title twice.
- Failures never leave a title stuck. The grab row records the error, a bad release goes on the blocklist, and the title goes back to `wanted` with a reason.
- An `available` title can be grabbed again (manual pick or upgrade).
- `watched` is not a state: it is per user, in `user_title_data`.
- A show's displayed state comes from its episodes: available once every aired, followed episode is, on the way while any is moving.

### 2.3 Other tables

| Table | Purpose |
|---|---|
| `users`, `sessions`, `api_keys` | accounts (argon2id), login sessions, hashed keys for other apps |
| `requests` | who asked for which title, status, decision, note. Many users can request the same title |
| `quality_profiles` | ordered allowed qualities, cutoff, size limits per minute, preferred words |
| `indexers`, `indexer_status` | Torznab/Newznab endpoints with cached capabilities; backoff and errors |
| `download_clients` | qBittorrent, Transmission, SABnzbd connections |
| `grabs` | one row per release sent to a client: title, release name, quality, client, download id, status, output path |
| `grab_titles` | other episodes a grab holds (season packs, multi-episode files) |
| `blocklist` | releases that failed |
| `media_files` | files in the library with probe results |
| `user_title_data` | per user: position, played |
| `settings`, `webhooks` | key/value settings (secrets sealed with `secret.key`), outgoing notifications |

Secrets are write-only through the API and encrypted at rest.

### 2.4 Quality

- **Video**: source (`cam` … `bluray`), resolution (`480` … `2160`), modifier (`remux`, `disc`, …) and a revision (proper/repack count). Keys look like `webdl-1080p`.
- **Audio**: `mp3-low`, `mp3`, `aac`, `opus`, `mp3-256`, `mp3-v0`, `mp3-320`, `alac`, `flac`, `flac-24`, worst to best.
- Order between qualities comes from the profile the user edits. Size limits are MB per minute of runtime, so one rule fits a short episode and a long film.

---

## 3. Crates

| Crate | Responsibility |
|---|---|
| `moka-core` | shared types: title kinds and states, video and audio quality, permissions |
| `moka-db` | SQLx pool, migrations (with a backup before each upgrade), every SQL query, secret sealing |
| `moka-parser` | release names: films, episodes (`S01E02`, `1x02`, season packs, air dates, absolute numbers), albums |
| `moka-metadata` | TMDB (films, shows, seasons) and MusicBrainz (artists, albums, tracks) clients |
| `moka-indexers` | Torznab/Newznab: capabilities, film and TV search, feeds, `.torrent`/magnet fetching, backoff |
| `moka-download` | qBittorrent, Transmission, SABnzbd behind one client type |
| `moka-decision` | accept/reject rules and ranking, for films, episodes, season packs and albums (by audio format) |
| `moka-net` | outgoing HTTP guard: blocks link-local, metadata and other forbidden addresses on every hop |
| `moka-server` | the binary: API, auth, background loop, import, playback, compatibility layer, embedded web app |
| `web/` | React app |

Crate names carry the working project name and will be renamed together with the product.

- Leaf crates know nothing about the database; they take inputs and return plain data, and are tested alone against local mock servers.
- Only `moka-db` writes SQL. Only `moka-server` wires things together.

Main modules in `moka-server`: `pipeline` (film loop, tracking, film import), `series` (episode search, season packs, episode import, show refresh), `scan` (bringing existing folders in), `streaming` and `subtitles` (playback), `notify` (webhooks), `jellyfin` and `radarr` (working next to those servers), `api/*` (routes).

---

## 4. Background work

One loop looks at the database and moves titles one step forward. Every step is idempotent, so a restart at any moment is safe.

```
every pass (a few seconds while something moves, a minute when idle, at once when woken):
  grabs in flight                → ask the client by download id, update progress, detect completion/failure
  titles in importing            → import (film, episode or season pack), then available
  wanted films, due              → search, decide, grab or reschedule
  wanted aired episodes, due     → per season: the whole season as one pack when nothing of it is in the
                                   library yet, else one episode; daily shows by air date, anime also by
                                   absolute number
```

- Fruitless searches back off: 15 min, 1 h, 3 h, 6 h, 12 h, then daily. A request or a manual action wakes the loop.
- Shows are refreshed from TMDB twice a day, so new episodes and air dates arrive on their own.
- Progress goes to the live event stream; only state changes are written.

## 5. Release selection

1. **Search** every usable indexer in parallel, by id when the indexer supports it, by text otherwise. TV searches use `season`/`ep` (or year and month/day for daily shows) and TV categories; film searches use film categories.
2. **Parse** each release name.
3. **Match**: indexer ids when present, else normalized title (+ year ±1). For shows: the season and episodes must be the ones wanted; a season pack only when the whole season is wanted.
4. **Rules**, each with a plain-language reason: quality in the profile, not an upgrade when the cutoff is met, size per minute within limits (episode count × runtime for packs), minimum seeders, not blocklisted, no burned-in subtitles, has a download link.
5. **Rank** accepted releases, first difference wins: profile order, revision, preferred words, indexer priority, seeders (log buckets), size closest to the preferred size.
6. **Grab** the best one. The UI shows every release with why it was accepted or not.

## 6. Download tracking and import

- The download id (torrent hash or Usenet job id) is saved at grab time; tracking is a lookup by id.
- **Placing files**: hardlink when the download and library are on the same filesystem (the torrent keeps seeding, no extra space), copy otherwise. Subtitles next to the video come along.
- **Films**: the biggest video (samples skipped) goes to `{films}/{Title} ({Year}) {tmdb-ID}/{Title} ({Year}) {tmdb-ID} - {Quality}.{ext}`.
- **Episodes**: every video in the download is matched to an episode by its name (season/episode, air date or absolute number) and placed at `{shows}/{Show} ({Year}) {tmdb-ID}/Season NN/{Show} - S01E02 - {Name} - {Quality}.{ext}`. Episodes the download did not hold go back to `wanted`.
- **Existing libraries**: a scan recognizes film folders and show folders (TMDB id in the name, else title + year) and album folders (`Artist/Album (Year)`, matched by name on MusicBrainz), and adds them as available, leaving the files in place.

## 7. Playback

The browser posts what it can decode; the server answers with one method and the reasons for it:

1. **Direct**: the file over HTTP with range requests.
2. **Remux**: streams copied into fragmented-MP4 HLS, audio converted only if needed.
3. **Convert**: video re-encoded (H.264), on the graphics card when configured (NVENC, VAAPI, Quick Sync), falling back to the processor if the card fails.

- HLS sessions start at the requested point; seeking outside the produced range restarts FFmpeg there. Idle sessions are reaped.
- **Subtitles**: text tracks (inside the file or next to it) are served as WebVTT and shifted with the stream; ASS/SSA are rendered in the browser with their styling and the fonts attached to the file; picture subtitles (PGS, VobSub) are drawn into the picture, which forces a conversion while they are on.

## 8. API

- **Native** `/api/v1`: JSON, `snake_case`, OpenAPI generated from the code; the web app uses a TypeScript client generated from it (CI fails on drift). Live updates on `GET /api/v1/events` (SSE).
- **Auth**: session cookie for the browser; API keys for other apps (`Authorization: Bearer`, `X-Api-Key` or `?apikey=`). Permissions are a bitmask: `admin`, `manage_requests`, `request`, `auto_approve`, `manage_library`, `play`. A test checks every route needs sign-in.
- **Main groups**: `auth`, `me`, `catalog` (films, shows, music), `requests`, `titles` (state, releases, grab, files, episodes/tracks, follow), `files` (stream, play, subtitles, fonts, progress), `hls`, `indexers`, `download-clients`, `quality-profiles`, `settings`, `webhooks`, `api-keys`, `library/scan`, `images` (TMDB artwork and album covers through a disk cache).
- **Compatibility** `/api/v3`: the routes that indexer managers, request apps and subtitle apps already speak for films and shows (indexers, profiles, root folders, movie/series lookup and add, episodes, episode files, queue, history, commands). Outgoing webhooks can also take the import formats those subtitle apps listen for.
- **Next**: a media-server compatible API so existing phone and TV apps can browse and play from Moka.

## 9. Configuration and files

- `MOKA_CONFIG_DIR` (default `./config`, `/config` in Docker): `moka.db`, `secret.key`, `cache/` (images, covers, probes, subtitles, fonts), transcode segments.
- `MOKA_PORT` (default `8484`), `MOKA_DATABASE_URL` (SQLite in the config dir by default).
- Everything else is set in the UI and stored in the database.
- Recommended Docker layout: one volume `/data` with downloads and library folders inside it, so hardlinks work. GPU passthrough examples are in `docker-compose.example.yml`.

## 10. Frontend

- React + Vite + TanStack Router + TanStack Query + Tailwind. The build is embedded in the binary.
- Pages: setup, login, discover (films/shows), search (films, shows, artists), film, show (seasons, per-episode panel), artist, album (tracks with play), requests, library, settings, player.
- Music player: one `<audio>` element in the app shell playing the album's files directly, with a queue, seeking and Media Session controls.
- Player: `<video>` + hls.js, JASSUB for styled subtitles, custom controls.
- The current look is a placeholder; a full redesign comes before 1.0.

## 11. Status and roadmap

**Done**: films end to end; security basics; working next to existing servers and apps; full browser playback; shows end to end (requests, season packs, daily and anime numbering, import, refresh, existing folders, compatibility layer); music end to end (catalog, album requests, search by audio format, import of tracks matched by number or title, playback in the browser).

**Next, in order**:
1. Music: bring in an existing music folder.
2. Ebooks and audiobooks.
3. Media-server compatible API for existing phone and TV apps.
4. Redesign, then 1.0.

## 12. Testing

- **Parser**: table-driven tests over real-world name shapes (films, episodes, albums).
- **Clients** (catalogs, indexers, download clients): local mock HTTP servers.
- **Decision**: pure functions, unit tests.
- **Database**: migrations on in-memory SQLite.
- **End to end**: a fake indexer and a fake qBittorrent carry a film and a season pack from wanted to the library; API tests drive requests, the compatibility layer and sign-in on every route.
- **FFmpeg**: tests marked `ignored` with `ffmpeg` in the name run real conversions (in CI and in Docker locally).
