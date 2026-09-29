-- The Move journal (KTD9). A change of storage folder with Move opens the
-- journal: one row for the Move, and one row for each note that it moves.
-- The Move records each step of a note here before it takes the next step,
-- so a crash resumes from what is on disk. The journal closes when every
-- note is done: then both tables are empty.
--
-- state is 'running' or 'paused'. pause is the JSON of the cause of a pause.
-- cross_volume is NULL until the Move finds whether the two folders are on
-- one volume.
CREATE TABLE move_journal (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    old_folder      TEXT NOT NULL,
    new_folder      TEXT NOT NULL,
    cross_volume    INTEGER,
    state           TEXT NOT NULL,
    pause           TEXT
);

-- step is the JSON of the note's step: pending, copied, placed, homed or
-- done.
CREATE TABLE move_notes (
    session_id      TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    step            TEXT NOT NULL
);
