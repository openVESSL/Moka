-- Request limits per user, films and shows apart (movie_quota_* exist since 0001):
-- NULL uses the global limit, 0 means no limit for this user.
ALTER TABLE users ADD COLUMN show_quota_limit INTEGER;
ALTER TABLE users ADD COLUMN show_quota_days INTEGER;
-- Whether the Discord bot may message the user when their title is ready.
ALTER TABLE users ADD COLUMN discord_notify INTEGER NOT NULL DEFAULT 1;
