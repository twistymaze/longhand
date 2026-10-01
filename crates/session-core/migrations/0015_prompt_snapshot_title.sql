-- The title text of the summary prompt snapshot (KTD7 of the title
-- instructions plan). Empty for a note from before this migration: its
-- title turn uses only the fixed title rules.
ALTER TABLE sessions ADD COLUMN prompt_snapshot_title_text TEXT NOT NULL DEFAULT '';
