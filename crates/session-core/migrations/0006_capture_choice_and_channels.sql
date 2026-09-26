-- The capture choice of a note, and the channel count of each segment
-- (KTD2, KTD6 of the system audio plan).
--
-- mic_id and mic_label are the microphone that the note chose: cpal's stable
-- device ID and the device name. Both are NULL for "System default", and for
-- a note from before this migration.
ALTER TABLE sessions ADD COLUMN mic_id TEXT;
ALTER TABLE sessions ADD COLUMN mic_label TEXT;
-- 1 while the note's Computer audio switch is on. NULL is off.
ALTER TABLE sessions ADD COLUMN computer_audio INTEGER;
-- The index of the first stereo segment of the note. NULL while every
-- segment is mono.
ALTER TABLE sessions ADD COLUMN stereo_from INTEGER;
-- 1 for a mono segment, 2 for a stereo segment. Every segment before this
-- migration is mono.
ALTER TABLE segments ADD COLUMN channels INTEGER NOT NULL DEFAULT 1;
