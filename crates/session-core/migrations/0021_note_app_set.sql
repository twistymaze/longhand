-- The app set of a note (R21, KTD7 of the app audio plan).
--
-- A JSON array of the apps whose audio the note records. Each item has the
-- app ID, the display name and the path of the app. NULL is an empty set:
-- every note before this migration has no apps.
ALTER TABLE sessions ADD COLUMN note_apps TEXT;
