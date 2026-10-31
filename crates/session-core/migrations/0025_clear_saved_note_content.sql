-- Clear the note content that earlier builds kept for saved notes (KTD12,
-- KTD15 of the transcript removal plan).
--
-- For each note in the Saved state, delete the rows that the save now
-- clears (`clear_recognition_data_in` in `jobs.rs`): the Assemble,
-- Transcribe and Diarize checkpoints, each `previous_*` checkpoint, and
-- the recognition output and epochs. `recognition_snapshot`,
-- `found_speakers` and the fingerprints stay.
DELETE FROM checkpoints
WHERE note_id IN (
        SELECT id FROM sessions WHERE json_extract(state, '$.state') = 'saved'
    )
    AND stage IN (
        'assemble', 'transcribe', 'diarize',
        'previous_assemble', 'previous_diarize', 'previous_transcribe',
        'previous_join', 'previous_verify', 'previous_summarize',
        'previous_save'
    );
DELETE FROM recognition_output
WHERE note_id IN (
    SELECT id FROM sessions WHERE json_extract(state, '$.state') = 'saved'
);
DELETE FROM recognition_epochs
WHERE note_id IN (
    SELECT id FROM sessions WHERE json_extract(state, '$.state') = 'saved'
);
-- The summary cache (`clear_summary_cache_in` in `jobs.rs`): the Summarize
-- checkpoints except the `failed` row. A note with a `title-request` row
-- has unfinished work, so its rows stay until the title job ends it.
DELETE FROM checkpoints
WHERE note_id IN (
        SELECT s.id FROM sessions s
        WHERE json_extract(s.state, '$.state') = 'saved'
            AND NOT EXISTS (
                SELECT 1 FROM checkpoints t
                WHERE t.note_id = s.id
                    AND t.stage = 'summarize'
                    AND t.key = 'title-request'
            )
    )
    AND stage = 'summarize'
    AND key != 'failed';
-- The next open runs one VACUUM, which removes the free pages.
UPDATE computer SET vacuum_owed = 1;
