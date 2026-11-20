-- Books: an author row and its ebooks and audiobooks (parent = author),
-- keyed by Open Library ids (`OL…W` for a book, `OL…A` for an author).
ALTER TABLE titles ADD COLUMN olid TEXT;
CREATE UNIQUE INDEX titles_kind_olid ON titles(kind, olid) WHERE olid IS NOT NULL;
