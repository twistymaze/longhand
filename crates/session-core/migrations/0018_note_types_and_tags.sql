-- The note type and the tags of a note (KTD1, KTD2, KTD10 of the note
-- types and tags plan).
--
-- The note type: the name of the type folder that holds the home folder.
-- Each library scan sets it from the path. NULL is "No type": the home
-- folder is directly in the storage folder. NULL for each row from before
-- this migration.
ALTER TABLE sessions ADD COLUMN note_type TEXT;
-- The tags of the note, as a JSON list of the exact texts in
-- `longhand_tags` of `session.md`. Each refresh sets it from the file.
-- An empty list for each row from before this migration.
ALTER TABLE sessions ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
-- The `longhand_pending_type` of `session.md` as JSON: a type change that
-- waits for its move. NULL when no move waits.
ALTER TABLE sessions ADD COLUMN pending_type TEXT;
