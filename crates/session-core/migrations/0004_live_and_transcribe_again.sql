-- The live mode that a session records when it starts, and the marks of a
-- Transcribe Again run (KTD2, KTD14).
--
-- live_transcript is 1 when the session's model streams and the Live
-- transcript switch was on at the start. It is NULL for a session from
-- before this migration.
ALTER TABLE sessions ADD COLUMN live_transcript INTEGER;
-- The model that a queued Transcribe Again run uses. NULL when the session
-- is not a Transcribe Again run. asr_model changes to it only after Save.
ALTER TABLE sessions ADD COLUMN transcribe_again_model TEXT;
-- The error of the last Transcribe Again run, for the note page. NULL when
-- the last run did not fail.
ALTER TABLE sessions ADD COLUMN transcribe_again_error TEXT;
