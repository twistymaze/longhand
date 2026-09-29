-- The active summary prompt when the note started recording (KTD15). The
-- first automatic summary uses it, so a change of the active prompt during
-- the recording does not change that summary. NULL for a note from before
-- this migration, and for a note that another computer recorded.
ALTER TABLE sessions ADD COLUMN prompt_snapshot_id TEXT;
ALTER TABLE sessions ADD COLUMN prompt_snapshot_name TEXT;
ALTER TABLE sessions ADD COLUMN prompt_snapshot_text TEXT;
