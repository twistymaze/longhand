-- The recording settings snapshot and the long-silence cut
-- (KTD14, KTD19, KTD21 of the forgotten-recording plan).
--
-- recording_guards: JSON with the four recording settings that the
-- current recording read at its start. NULL for a note that has no
-- snapshot: such a note gets no checks and no cut.
--
-- cut_before_ms, cut_after_ms: the removed audio before the segment start
-- and after the segment end. 0 for a segment with no cut.
--
-- cut_checked: 1 when the cut checked the segment. Every segment before
-- this migration is 0.
ALTER TABLE sessions ADD COLUMN recording_guards TEXT;
ALTER TABLE segments ADD COLUMN cut_before_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE segments ADD COLUMN cut_after_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE segments ADD COLUMN cut_checked INTEGER NOT NULL DEFAULT 0;
