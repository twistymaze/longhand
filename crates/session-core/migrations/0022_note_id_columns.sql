-- The note id column (R5, KTD4 of the note id plan).
--
-- Each column named session_id gets the name note_id. The rows, the
-- primary keys and the foreign keys stay.
ALTER TABLE segments RENAME COLUMN session_id TO note_id;
ALTER TABLE jobs RENAME COLUMN session_id TO note_id;
ALTER TABLE checkpoints RENAME COLUMN session_id TO note_id;
ALTER TABLE rename_intents RENAME COLUMN session_id TO note_id;
ALTER TABLE owed_renames RENAME COLUMN session_id TO note_id;
ALTER TABLE recognition_output RENAME COLUMN session_id TO note_id;
ALTER TABLE recognition_epochs RENAME COLUMN session_id TO note_id;
ALTER TABLE result_state RENAME COLUMN session_id TO note_id;
ALTER TABLE move_notes RENAME COLUMN session_id TO note_id;
ALTER TABLE type_moves RENAME COLUMN session_id TO note_id;
