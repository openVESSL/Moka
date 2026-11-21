-- The Discord account of a user, so the Discord bot can request in their name.
ALTER TABLE users ADD COLUMN discord_id TEXT;
CREATE UNIQUE INDEX users_discord_id ON users(discord_id) WHERE discord_id IS NOT NULL;

-- When a title last arrived through a download (not a library scan), so the
-- Discord bot can catch up after being offline and build its weekly summary.
ALTER TABLE titles ADD COLUMN available_at INTEGER;
CREATE INDEX titles_available_at ON titles(available_at) WHERE available_at IS NOT NULL;
