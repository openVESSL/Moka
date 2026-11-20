-- Shows: a show row, its seasons (parent = show) and episodes (parent = season).
ALTER TABLE titles ADD COLUMN season_number INTEGER;
ALTER TABLE titles ADD COLUMN episode_number INTEGER;
ALTER TABLE titles ADD COLUMN tvdb_id INTEGER;
CREATE INDEX titles_parent ON titles(parent_id);
