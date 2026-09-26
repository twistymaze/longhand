-- The fingerprint of the transcript.md that this computer last wrote or read
-- for the note (KTD12 of the one-library plan). When the folder holds a
-- transcript.md with another fingerprint, another computer wrote it, and this
-- computer's stored recognition results for the note do not apply now (R18).
-- NULL for a note from before this migration.
ALTER TABLE sessions ADD COLUMN transcript_fingerprint TEXT;
