-- Episodes a grab holds besides its own title: a season pack, or a release
-- with several episodes in one file.
CREATE TABLE grab_titles (
    grab_id         INTEGER NOT NULL REFERENCES grabs(id) ON DELETE CASCADE,
    title_id        INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    PRIMARY KEY (grab_id, title_id)
);
CREATE INDEX grab_titles_title ON grab_titles(title_id);
