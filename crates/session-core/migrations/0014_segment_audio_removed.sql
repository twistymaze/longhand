-- A segment whose note folder has the removal marker of its audio file
-- (KTD1, KTD9 of the audio removal plan). The scan, the refresh and the
-- adopt set it from the folder. 0 for each segment from before this
-- migration: the next scan sets the flag from the folder.
ALTER TABLE segments ADD COLUMN audio_removed INTEGER NOT NULL DEFAULT 0;
