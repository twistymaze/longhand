-- Titles, the notes document and the found speaker count (KTD5, KTD7, KTD11).
--
-- A title has a source: `proposed` by processing or `user`. A proposal
-- never replaces a title that the user set (R31).
ALTER TABLE sessions ADD COLUMN title TEXT;
ALTER TABLE sessions ADD COLUMN title_source TEXT CHECK (title_source IN ('proposed', 'user'));
-- The session's notes as one markdown document, in the form the user typed
-- them (R25).
ALTER TABLE sessions ADD COLUMN notes TEXT NOT NULL DEFAULT '';
-- The number of speakers that processing found. The user's bound on the
-- speaker count (R36) stays in speaker_count.
ALTER TABLE sessions ADD COLUMN found_speakers INTEGER;

-- Notes typed while no session was open are not kept (R27).
DELETE FROM notes WHERE session_id IS NULL;

-- The hook in schema.rs then writes each session's note rows into its notes
-- document and drops the notes table.
