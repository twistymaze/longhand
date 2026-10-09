-- The pause mark of a segment (KTD4 of the pause plan).
--
-- 1 when the note paused after this segment. Pause sets it on the note's
-- last kept segment. A resume of the note clears it on the note's last
-- segment. Every segment before this migration is 0.
ALTER TABLE segments ADD COLUMN ends_in_pause INTEGER NOT NULL DEFAULT 0;
