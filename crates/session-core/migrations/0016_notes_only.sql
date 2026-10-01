-- Notes that open with no recording (KTD2, KTD8 of the plan for notes
-- without a recording).
--
-- The time the note opened (R16). The store writes it for a note that
-- opens with no recording, and for an adopted note with no segments. The
-- note keeps it when it records later. NULL for every other note: its start
-- is its first segment. NULL for each row from before this migration.
ALTER TABLE sessions ADD COLUMN started_at TEXT;
-- 1 when this computer started the note. Only such a note can be removed
-- as an empty note (R18). 0 for each row from before this migration: this
-- computer cannot tell, so removal keeps the note.
ALTER TABLE sessions ADD COLUMN started_here INTEGER NOT NULL DEFAULT 0;
-- The fingerprints of the `session.md` and `notes.md` that this computer
-- last wrote into the note's home folder (R18). Removal compares them with
-- the files on disk. NULL until this computer writes the file.
ALTER TABLE sessions ADD COLUMN written_session_fingerprint TEXT;
ALTER TABLE sessions ADD COLUMN written_notes_fingerprint TEXT;
