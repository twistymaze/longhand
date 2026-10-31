-- The removal of transcripts (KTD5, KTD9 of the transcript removal plan).
--
-- 1 when the note's home folder has the marker `transcript.removed`. The
-- scan, the refresh and the adopt set it from the folder. 0 for each row
-- from before this migration: the next scan sets the flag from the folder.
ALTER TABLE sessions ADD COLUMN transcript_removed INTEGER NOT NULL DEFAULT 0;
-- 1 when the store owes one VACUUM. `Store::open` runs it outside a
-- transaction, then sets 0. An earlier build deleted rows with no
-- `secure_delete`, so free pages can still hold their text. The VACUUM
-- removes those free pages. A later migration that deletes such rows can
-- set 1 again.
ALTER TABLE computer ADD COLUMN vacuum_owed INTEGER NOT NULL DEFAULT 1;
