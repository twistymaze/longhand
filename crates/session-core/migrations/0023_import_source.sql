-- The import source of a note (KTD5, KTD13 of the audio import plan).
--
-- JSON with the name of the imported file and, when the copy is
-- complete, its SHA-256. NULL for a recorded note. A row that has it is an
-- imported note: the queue takes each recorded note before it.
--
-- The migration also raises the version above the version of each older
-- build. An older build cannot read the state `importing`, so it refuses
-- the database and rebuilds it.
ALTER TABLE sessions ADD COLUMN import_source TEXT;
