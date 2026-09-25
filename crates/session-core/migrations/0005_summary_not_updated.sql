-- Why the last Transcribe Again run saved a new transcript and kept the
-- previous summary (R19, KTD14). NULL when the summary was updated, or when
-- no Transcribe Again run saved.
ALTER TABLE sessions ADD COLUMN transcribe_again_summary_error TEXT;
