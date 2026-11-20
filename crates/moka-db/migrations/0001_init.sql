-- Initial schema. Timestamps are unix seconds (UTC).

CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    password_hash   TEXT    NOT NULL,
    permissions     INTEGER NOT NULL DEFAULT 0,
    movie_quota_limit INTEGER,
    movie_quota_days  INTEGER,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE sessions (
    token_hash      TEXT    PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at      INTEGER NOT NULL,
    user_agent      TEXT
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE api_keys (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT    NOT NULL,
    token_hash      TEXT    NOT NULL UNIQUE,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    last_used_at    INTEGER
);

CREATE TABLE settings (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL
);

CREATE TABLE quality_profiles (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL UNIQUE,
    -- JSON array of quality keys, best first, e.g. ["bluray-1080p","webdl-1080p"]
    qualities       TEXT    NOT NULL,
    cutoff          TEXT    NOT NULL,
    upgrades_allowed INTEGER NOT NULL DEFAULT 1,
    min_seeders     INTEGER NOT NULL DEFAULT 1,
    -- JSON object: quality key -> { "min": MB/min, "max": MB/min | null, "preferred": MB/min | null }
    size_limits     TEXT    NOT NULL DEFAULT '{}',
    -- JSON array of { "term": "...", "score": n } added to the ranking score
    preferred_terms TEXT    NOT NULL DEFAULT '[]'
);

CREATE TABLE titles (
    id              INTEGER PRIMARY KEY,
    kind            TEXT    NOT NULL,
    parent_id       INTEGER REFERENCES titles(id) ON DELETE CASCADE,
    tmdb_id         INTEGER,
    imdb_id         TEXT,
    title           TEXT    NOT NULL,
    original_title  TEXT,
    year            INTEGER,
    overview        TEXT,
    runtime_minutes INTEGER,
    release_date    TEXT,
    digital_release_date  TEXT,
    physical_release_date TEXT,
    poster_path     TEXT,
    backdrop_path   TEXT,
    logo_path       TEXT,
    metadata_json   TEXT,
    state           TEXT    NOT NULL,
    state_reason    TEXT,
    quality_profile_id INTEGER REFERENCES quality_profiles(id),
    monitored       INTEGER NOT NULL DEFAULT 1,
    next_search_at  INTEGER,
    last_search_at  INTEGER,
    search_attempts INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at      INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE UNIQUE INDEX titles_kind_tmdb ON titles(kind, tmdb_id) WHERE tmdb_id IS NOT NULL;
CREATE INDEX titles_state ON titles(state);

CREATE TABLE requests (
    id              INTEGER PRIMARY KEY,
    title_id        INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status          TEXT    NOT NULL,           -- pending | approved | declined
    decided_by      INTEGER REFERENCES users(id) ON DELETE SET NULL,
    decided_at      INTEGER,
    note            TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE (title_id, user_id)
);
CREATE INDEX requests_status ON requests(status);

CREATE TABLE indexers (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    kind            TEXT    NOT NULL,           -- torznab | newznab
    base_url        TEXT    NOT NULL,
    api_path        TEXT    NOT NULL DEFAULT '/api',
    api_key         TEXT,
    categories      TEXT    NOT NULL DEFAULT '[2000]',
    priority        INTEGER NOT NULL DEFAULT 25,
    enabled         INTEGER NOT NULL DEFAULT 1,
    capabilities    TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE indexer_status (
    indexer_id      INTEGER PRIMARY KEY REFERENCES indexers(id) ON DELETE CASCADE,
    failure_level   INTEGER NOT NULL DEFAULT 0,
    disabled_until  INTEGER,
    last_error      TEXT,
    last_failure_at INTEGER
);

CREATE TABLE download_clients (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    kind            TEXT    NOT NULL,           -- qbittorrent
    base_url        TEXT    NOT NULL,
    username        TEXT,
    password        TEXT,
    category        TEXT    NOT NULL DEFAULT 'moka',
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE grabs (
    id              INTEGER PRIMARY KEY,
    title_id        INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    indexer_id      INTEGER REFERENCES indexers(id) ON DELETE SET NULL,
    download_client_id INTEGER REFERENCES download_clients(id) ON DELETE SET NULL,
    release_title   TEXT    NOT NULL,
    size_bytes      INTEGER,
    quality         TEXT,
    info_hash       TEXT,
    status          TEXT    NOT NULL,           -- sent | downloading | completed | imported | failed
    error           TEXT,
    output_path     TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at      INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX grabs_title ON grabs(title_id);
CREATE INDEX grabs_hash ON grabs(info_hash);

CREATE TABLE blocklist (
    id              INTEGER PRIMARY KEY,
    title_id        INTEGER REFERENCES titles(id) ON DELETE CASCADE,
    release_title   TEXT    NOT NULL,
    info_hash       TEXT,
    reason          TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX blocklist_hash ON blocklist(info_hash);

CREATE TABLE media_files (
    id              INTEGER PRIMARY KEY,
    title_id        INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    path            TEXT    NOT NULL UNIQUE,
    size_bytes      INTEGER NOT NULL,
    quality         TEXT,
    release_group   TEXT,
    duration_seconds REAL,
    probe_json      TEXT,
    added_at        INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX media_files_title ON media_files(title_id);

CREATE TABLE user_title_data (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title_id        INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    position_seconds REAL   NOT NULL DEFAULT 0,
    played          INTEGER NOT NULL DEFAULT 0,
    play_count      INTEGER NOT NULL DEFAULT 0,
    favorite        INTEGER NOT NULL DEFAULT 0,
    last_played_at  INTEGER,
    PRIMARY KEY (user_id, title_id)
);

CREATE TABLE events (
    id              INTEGER PRIMARY KEY,
    kind            TEXT    NOT NULL,
    title_id        INTEGER REFERENCES titles(id) ON DELETE CASCADE,
    user_id         INTEGER REFERENCES users(id) ON DELETE SET NULL,
    data            TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX events_created ON events(created_at);

CREATE TABLE webhooks (
    id              INTEGER PRIMARY KEY,
    name            TEXT    NOT NULL,
    url             TEXT    NOT NULL,
    events          TEXT    NOT NULL DEFAULT '["requested","grabbed","available","failed"]',
    enabled         INTEGER NOT NULL DEFAULT 1
);

INSERT INTO quality_profiles (name, qualities, cutoff, size_limits) VALUES (
    'HD 1080p',
    '["bluray-1080p","webdl-1080p","webrip-1080p","bluray-720p","webdl-720p","webrip-720p","tv-1080p","tv-720p"]',
    'bluray-1080p',
    '{"bluray-1080p":{"min":10,"max":200,"preferred":95},"webdl-1080p":{"min":5,"max":120,"preferred":60},"webrip-1080p":{"min":5,"max":120,"preferred":60},"bluray-720p":{"min":5,"max":100,"preferred":45},"webdl-720p":{"min":3,"max":80,"preferred":30},"webrip-720p":{"min":3,"max":80,"preferred":30}}'
);
