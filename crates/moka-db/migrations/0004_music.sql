-- Music: an artist row, its albums (parent = artist) and their tracks
-- (parent = album; disc in season_number, track in episode_number).
ALTER TABLE titles ADD COLUMN mbid TEXT;
CREATE UNIQUE INDEX titles_kind_mbid ON titles(kind, mbid) WHERE mbid IS NOT NULL;
